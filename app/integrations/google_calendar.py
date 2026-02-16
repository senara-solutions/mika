"""Google Calendar integration using OAuth 2.0."""

from datetime import datetime, timedelta

from google.oauth2.credentials import Credentials
from googleapiclient.discovery import build
from sqlalchemy import select

from app.common.db import async_session_factory
from app.common.logging import get_logger
from app.config import settings
from app.models.user import User

logger = get_logger(__name__)

SCOPES = ["https://www.googleapis.com/auth/calendar.readonly"]


def get_calendar_service(credentials_data: dict):
    """Build a Google Calendar API service from stored credentials."""
    creds = Credentials(
        token=credentials_data["access_token"],
        refresh_token=credentials_data.get("refresh_token"),
        token_uri="https://oauth2.googleapis.com/token",
        client_id=settings.google_client_id,
        client_secret=settings.google_client_secret,
    )
    return build("calendar", "v3", credentials=creds, cache_discovery=False)


def get_todays_events(service, timezone: str = "UTC") -> list[dict]:
    """Fetch today's calendar events."""
    now = datetime.now()
    start_of_day = now.replace(hour=0, minute=0, second=0, microsecond=0)
    end_of_day = start_of_day + timedelta(days=1)

    events_result = (
        service.events()
        .list(
            calendarId="primary",
            timeMin=start_of_day.isoformat() + "Z",
            timeMax=end_of_day.isoformat() + "Z",
            singleEvents=True,
            orderBy="startTime",
            timeZone=timezone,
        )
        .execute()
    )

    events = []
    for event in events_result.get("items", []):
        start = event["start"].get("dateTime", event["start"].get("date"))
        events.append(
            {
                "summary": event.get("summary", "No title"),
                "start": start,
                "end": event["end"].get("dateTime", event["end"].get("date")),
                "location": event.get("location", ""),
                "attendees": [
                    a.get("email", "")
                    for a in event.get("attendees", [])
                ],
                "description": event.get("description", ""),
            }
        )
    return events


def format_events_for_briefing(events: list[dict]) -> str:
    """Format calendar events as text for morning briefing."""
    if not events:
        return "No meetings scheduled for today."

    lines = [f"You have {len(events)} event(s) today:\n"]
    for event in events:
        time_str = event["start"]
        # Try to extract just the time portion
        if "T" in time_str:
            time_str = time_str.split("T")[1][:5]

        line = f"- {time_str}: {event['summary']}"
        if event.get("location"):
            line += f" ({event['location']})"
        if event.get("attendees"):
            names = [a.split("@")[0] for a in event["attendees"][:3]]
            line += f" with {', '.join(names)}"
            if len(event["attendees"]) > 3:
                line += f" +{len(event['attendees']) - 3} others"
        lines.append(line)

    return "\n".join(lines)


async def get_user_calendar_data(user_id: str) -> str | None:
    """Get formatted calendar data for a user's morning briefing."""
    async with async_session_factory() as session:
        result = await session.execute(
            select(User).where(User.id == user_id)
        )
        user = result.scalar_one_or_none()
        if not user or not user.google_credentials:
            return None

    try:
        service = get_calendar_service(user.google_credentials)
        events = get_todays_events(service, timezone=user.timezone)
        return format_events_for_briefing(events)
    except Exception:
        logger.debug("Could not fetch calendar data", extra={"user_id": user_id})
        return None
