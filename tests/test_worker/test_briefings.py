"""Tests for morning briefing task."""

import uuid
from unittest.mock import AsyncMock, MagicMock, patch

from langchain_core.messages import AIMessage

from app.worker.tasks.briefings import (
    BRIEFING_PROMPT,
    _compose_briefing,
    _get_briefing_eligible_users,
)


class TestBriefingPrompt:
    def test_prompt_includes_memory_placeholder(self):
        assert "{memory_context}" in BRIEFING_PROMPT

    def test_prompt_asks_for_greeting(self):
        assert "Greets them warmly" in BRIEFING_PROMPT


class TestComposeBriefing:
    def test_compose_briefing_calls_llm(self):
        mock_llm = MagicMock()
        mock_llm.ainvoke = AsyncMock(
            return_value=AIMessage(
                content="Good morning! Here's your day..."
            )
        )

        with (
            patch(
                "app.common.llm.get_sonnet",
                return_value=mock_llm,
            ),
            patch(
                "app.worker.tasks.briefings.asyncio"
            ) as mock_asyncio,
        ):
            mock_loop = MagicMock()
            mock_loop.run_until_complete = MagicMock(
                return_value="Good morning! Here's your day..."
            )
            mock_asyncio.get_event_loop.return_value = mock_loop

            result = _compose_briefing("### Commitments\n- Proposal due")
            assert result == "Good morning! Here's your day..."

    def test_compose_briefing_returns_empty_on_failure(self):
        with patch(
            "app.worker.tasks.briefings.asyncio"
        ) as mock_asyncio:
            mock_loop = MagicMock()
            mock_loop.run_until_complete = MagicMock(
                side_effect=Exception("LLM failed")
            )
            mock_asyncio.get_event_loop.return_value = mock_loop

            result = _compose_briefing("some context")
            assert result == ""


class TestBriefingEligibility:
    def test_eligible_users_filters_by_time_window(self):
        mock_session = MagicMock()
        mock_result = MagicMock()
        mock_result.all.return_value = [
            MagicMock(
                id=uuid.uuid4(),
                timezone="America/New_York",
                preferred_channel="telegram",
                onboarding_completed=True,
            ),
        ]
        mock_session.execute.return_value = mock_result
        mock_session.__enter__ = MagicMock(return_value=mock_session)
        mock_session.__exit__ = MagicMock(return_value=False)

        with (
            patch(
                "app.common.db.sync_engine",
                return_value=MagicMock(),
            ),
            patch(
                "app.common.timezone.is_in_time_window",
                return_value=True,
            ),
        ):
            # Patch Session at the source where it's imported
            with patch(
                "sqlalchemy.orm.Session",
                return_value=mock_session,
            ):
                users = _get_briefing_eligible_users()
                assert len(users) == 1
                assert users[0]["timezone"] == "America/New_York"

    def test_eligible_users_empty_on_failure(self):
        with patch(
            "app.common.db.sync_engine",
            side_effect=Exception("DB down"),
        ):
            users = _get_briefing_eligible_users()
            assert users == []
