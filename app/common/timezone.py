"""Timezone utilities for proactive messaging."""

from datetime import UTC, datetime
from zoneinfo import ZoneInfo

DEFAULT_TIMEZONE = "UTC"
ACTIVE_HOURS_START = 8  # 8 AM
ACTIVE_HOURS_END = 21  # 9 PM


def user_local_now(tz_name: str = DEFAULT_TIMEZONE) -> datetime:
    """Get current time in user's timezone."""
    try:
        tz = ZoneInfo(tz_name)
    except (KeyError, ValueError):
        tz = UTC
    return datetime.now(tz)


def is_active_hours(
    tz_name: str = DEFAULT_TIMEZONE,
    start: int = ACTIVE_HOURS_START,
    end: int = ACTIVE_HOURS_END,
) -> bool:
    """Check if it's within active hours in the user's timezone."""
    local = user_local_now(tz_name)
    return start <= local.hour < end


def is_in_time_window(
    tz_name: str,
    window_start_hour: int,
    window_start_minute: int,
    window_end_hour: int,
    window_end_minute: int,
) -> bool:
    """Check if user's local time falls within a specific window."""
    local = user_local_now(tz_name)
    current_minutes = local.hour * 60 + local.minute
    start_minutes = window_start_hour * 60 + window_start_minute
    end_minutes = window_end_hour * 60 + window_end_minute
    return start_minutes <= current_minutes < end_minutes
