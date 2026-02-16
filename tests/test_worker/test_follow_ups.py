"""Tests for follow-up tasks, nudge composition, timezone, and rate limiting."""

from datetime import UTC, datetime, timedelta

import pytest

from app.worker.tasks.follow_ups import _compose_nudge


class TestComposeNudge:
    def test_overdue_commitment(self):
        yesterday = (datetime.now(UTC) - timedelta(days=1)).isoformat()
        result = _compose_nudge("Send proposal", yesterday)
        assert "overdue" in result
        assert "Send proposal" in result

    def test_due_today(self):
        today = datetime.now(UTC).isoformat()
        result = _compose_nudge("Review document", today)
        assert "due today" in result
        assert "Review document" in result

    def test_due_tomorrow(self):
        tomorrow = (datetime.now(UTC) + timedelta(days=1)).isoformat()
        result = _compose_nudge("Call client", tomorrow)
        assert "coming up" in result
        assert "1 day" in result

    def test_due_in_two_days(self):
        future = (datetime.now(UTC) + timedelta(days=2)).isoformat()
        result = _compose_nudge("Finish report", future)
        assert "coming up" in result
        assert "2 days" in result

    def test_no_due_date(self):
        result = _compose_nudge("General task", None)
        assert "checking in" in result
        assert "General task" in result

    def test_far_future_due_date(self):
        far = (datetime.now(UTC) + timedelta(days=30)).isoformat()
        result = _compose_nudge("Long term goal", far)
        # Falls through to generic message since > 2 days away
        assert "checking in" in result

    def test_invalid_due_date_falls_through(self):
        result = _compose_nudge("Task", "not-a-date")
        assert "checking in" in result


class TestTimezoneUtils:
    def test_user_local_now_utc(self):
        from app.common.timezone import user_local_now

        now = user_local_now("UTC")
        assert now.tzinfo is not None

    def test_user_local_now_invalid_tz(self):
        from app.common.timezone import user_local_now

        now = user_local_now("Invalid/Timezone")
        assert now.tzinfo is not None

    def test_is_active_hours_custom_window(self):
        from app.common.timezone import is_active_hours

        # This just tests it runs without error; actual result depends
        # on current time
        result = is_active_hours("UTC", start=0, end=24)
        assert result is True

    def test_is_active_hours_never_active(self):
        from app.common.timezone import is_active_hours

        # Window where start == end -- always false
        result = is_active_hours("UTC", start=25, end=25)
        assert result is False

    def test_is_in_time_window(self):
        from app.common.timezone import is_in_time_window

        # Full day window
        result = is_in_time_window("UTC", 0, 0, 23, 59)
        assert result is True


class TestRateLimiter:
    @pytest.mark.asyncio
    async def test_can_send_when_no_history(self):
        from unittest.mock import AsyncMock, patch

        mock_redis = AsyncMock()
        mock_redis.get = AsyncMock(return_value=None)
        mock_redis.aclose = AsyncMock()

        with patch(
            "app.common.rate_limiter._get_redis",
            return_value=mock_redis,
        ):
            from app.common.rate_limiter import can_send_proactive

            result = await can_send_proactive("user-123")
            assert result is True

    @pytest.mark.asyncio
    async def test_blocked_when_hourly_limit_hit(self):
        from unittest.mock import AsyncMock, patch

        mock_redis = AsyncMock()
        mock_redis.get = AsyncMock(side_effect=["1", "0"])
        mock_redis.aclose = AsyncMock()

        with patch(
            "app.common.rate_limiter._get_redis",
            return_value=mock_redis,
        ):
            from app.common.rate_limiter import can_send_proactive

            result = await can_send_proactive("user-123")
            assert result is False

    @pytest.mark.asyncio
    async def test_blocked_when_daily_limit_hit(self):
        from unittest.mock import AsyncMock, patch

        mock_redis = AsyncMock()
        mock_redis.get = AsyncMock(side_effect=["0", "3"])
        mock_redis.aclose = AsyncMock()

        with patch(
            "app.common.rate_limiter._get_redis",
            return_value=mock_redis,
        ):
            from app.common.rate_limiter import can_send_proactive

            result = await can_send_proactive("user-123")
            assert result is False

    @pytest.mark.asyncio
    async def test_record_proactive_send(self):
        from unittest.mock import AsyncMock, MagicMock, patch

        mock_pipe = MagicMock()
        mock_pipe.incr = MagicMock()
        mock_pipe.expire = MagicMock()
        mock_pipe.execute = AsyncMock()

        mock_redis = AsyncMock()
        mock_redis.pipeline = MagicMock(return_value=mock_pipe)
        mock_redis.aclose = AsyncMock()

        with patch(
            "app.common.rate_limiter._get_redis",
            return_value=mock_redis,
        ):
            from app.common.rate_limiter import record_proactive_send

            await record_proactive_send("user-123")
            assert mock_pipe.incr.call_count == 2
            assert mock_pipe.expire.call_count == 2
            mock_pipe.execute.assert_called_once()
