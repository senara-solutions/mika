"""Tests for Google Calendar integration."""

from unittest.mock import MagicMock, patch

from app.integrations.google_calendar import (
    format_events_for_briefing,
    get_todays_events,
)


class TestFormatEventsForBriefing:
    def test_no_events(self):
        result = format_events_for_briefing([])
        assert "No meetings" in result

    def test_single_event(self):
        events = [
            {
                "summary": "Team Standup",
                "start": "2026-02-16T09:00:00",
                "end": "2026-02-16T09:30:00",
                "location": "Zoom",
                "attendees": ["sarah@example.com", "john@example.com"],
                "description": "",
            }
        ]
        result = format_events_for_briefing(events)
        assert "1 event(s)" in result
        assert "Team Standup" in result
        assert "Zoom" in result
        assert "sarah" in result

    def test_multiple_events(self):
        events = [
            {
                "summary": "Morning Standup",
                "start": "2026-02-16T09:00:00",
                "end": "2026-02-16T09:15:00",
                "location": "",
                "attendees": [],
                "description": "",
            },
            {
                "summary": "Board Meeting",
                "start": "2026-02-16T14:00:00",
                "end": "2026-02-16T15:00:00",
                "location": "Conference Room A",
                "attendees": [
                    "ceo@example.com",
                    "cfo@example.com",
                    "cto@example.com",
                    "coo@example.com",
                ],
                "description": "Quarterly review",
            },
        ]
        result = format_events_for_briefing(events)
        assert "2 event(s)" in result
        assert "Morning Standup" in result
        assert "Board Meeting" in result
        assert "+1 others" in result

    def test_all_day_event(self):
        events = [
            {
                "summary": "Company Retreat",
                "start": "2026-02-16",
                "end": "2026-02-17",
                "location": "",
                "attendees": [],
                "description": "",
            }
        ]
        result = format_events_for_briefing(events)
        assert "Company Retreat" in result


class TestGetTodaysEvents:
    def test_fetches_and_parses_events(self):
        mock_service = MagicMock()
        mock_events = mock_service.events.return_value
        mock_list = mock_events.list.return_value
        mock_list.execute.return_value = {
            "items": [
                {
                    "summary": "Test Meeting",
                    "start": {"dateTime": "2026-02-16T10:00:00-05:00"},
                    "end": {"dateTime": "2026-02-16T11:00:00-05:00"},
                    "attendees": [{"email": "colleague@example.com"}],
                }
            ]
        }

        events = get_todays_events(mock_service, timezone="America/New_York")
        assert len(events) == 1
        assert events[0]["summary"] == "Test Meeting"
        assert events[0]["attendees"] == ["colleague@example.com"]

    def test_empty_calendar(self):
        mock_service = MagicMock()
        mock_events = mock_service.events.return_value
        mock_list = mock_events.list.return_value
        mock_list.execute.return_value = {"items": []}

        events = get_todays_events(mock_service)
        assert events == []


class TestGetCalendarService:
    def test_creates_service_from_credentials(self):
        from app.integrations.google_calendar import get_calendar_service

        creds_data = {
            "access_token": "ya29.test-token",
            "refresh_token": "1//test-refresh",
        }

        with patch(
            "app.integrations.google_calendar.build"
        ) as mock_build:
            mock_build.return_value = MagicMock()
            service = get_calendar_service(creds_data)
            mock_build.assert_called_once()
            assert service is not None
