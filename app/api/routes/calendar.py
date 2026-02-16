"""Google Calendar OAuth and dashboard routes."""

from fastapi import APIRouter, Request
from fastapi.responses import HTMLResponse, JSONResponse, RedirectResponse
from google_auth_oauthlib.flow import Flow
from sqlalchemy import select

from app.common.db import async_session_factory
from app.common.logging import get_logger
from app.config import settings
from app.integrations.google_calendar import SCOPES
from app.models.user import User

logger = get_logger(__name__)

router = APIRouter(prefix="/dashboard/calendar", tags=["calendar"])


def _get_oauth_flow(redirect_uri: str) -> Flow:
    """Create Google OAuth flow."""
    return Flow.from_client_config(
        {
            "web": {
                "client_id": settings.google_client_id,
                "client_secret": settings.google_client_secret,
                "auth_uri": "https://accounts.google.com/o/oauth2/auth",
                "token_uri": "https://oauth2.googleapis.com/token",
            }
        },
        scopes=SCOPES,
        redirect_uri=redirect_uri,
    )


@router.get("/connect")
async def connect_calendar(request: Request):
    """Start Google Calendar OAuth flow."""
    user = getattr(request.state, "user", None)
    if not user:
        return RedirectResponse("/login", status_code=303)

    if not settings.google_client_id:
        return JSONResponse(
            {"error": "Google Calendar not configured"}, status_code=503
        )

    redirect_uri = str(request.url_for("calendar_callback"))
    flow = _get_oauth_flow(redirect_uri)
    auth_url, _ = flow.authorization_url(
        access_type="offline",
        include_granted_scopes="true",
        prompt="consent",
        state=str(user.id),
    )

    return RedirectResponse(auth_url)


@router.get("/callback")
async def calendar_callback(request: Request):
    """Handle Google OAuth callback."""
    code = request.query_params.get("code")
    state = request.query_params.get("state")

    if not code or not state:
        return RedirectResponse(
            "/dashboard/settings?error=calendar_auth_failed",
            status_code=303,
        )

    redirect_uri = str(request.url_for("calendar_callback"))
    flow = _get_oauth_flow(redirect_uri)

    try:
        flow.fetch_token(code=code)
        credentials = flow.credentials

        creds_data = {
            "access_token": credentials.token,
            "refresh_token": credentials.refresh_token,
            "token_uri": credentials.token_uri,
            "scopes": list(credentials.scopes or []),
        }

        # Store credentials
        async with async_session_factory() as session:
            result = await session.execute(
                select(User).where(User.id == state)
            )
            user = result.scalar_one_or_none()
            if user:
                user.google_credentials = creds_data
                await session.commit()

        return RedirectResponse(
            "/dashboard/calendar?connected=1", status_code=303
        )
    except Exception:
        logger.exception("Google Calendar OAuth failed")
        return RedirectResponse(
            "/dashboard/settings?error=calendar_auth_failed",
            status_code=303,
        )


@router.get("/disconnect")
async def disconnect_calendar(request: Request):
    """Remove Google Calendar connection."""
    user = getattr(request.state, "user", None)
    if not user:
        return RedirectResponse("/login", status_code=303)

    async with async_session_factory() as session:
        result = await session.execute(
            select(User).where(User.id == user.id)
        )
        db_user = result.scalar_one_or_none()
        if db_user:
            db_user.google_credentials = None
            await session.commit()

    return RedirectResponse(
        "/dashboard/calendar?disconnected=1", status_code=303
    )


@router.get("", response_class=HTMLResponse)
async def calendar_page(request: Request):
    """Calendar integration dashboard page."""
    user = getattr(request.state, "user", None)
    if not user:
        return RedirectResponse("/login", status_code=303)

    connected = user.google_credentials is not None
    events = []

    if connected:
        try:
            from app.integrations.google_calendar import (
                get_calendar_service,
                get_todays_events,
            )

            service = get_calendar_service(user.google_credentials)
            events = get_todays_events(service, timezone=user.timezone)
        except Exception:
            logger.debug("Could not fetch calendar events for dashboard")

    return request.app.state.templates.TemplateResponse(
        "dashboard/calendar.html",
        {
            "request": request,
            "user": user,
            "connected": connected,
            "events": events,
            "google_configured": bool(settings.google_client_id),
        },
    )
