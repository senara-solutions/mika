"""Tests for conversation summarization maintenance task."""

from unittest.mock import MagicMock, patch

from app.worker.tasks.maintenance import (
    SUMMARIZATION_PROMPT,
    _format_conversations,
)


class TestSummarizationPrompt:
    def test_prompt_includes_conversation_placeholder(self):
        assert "{conversation_text}" in SUMMARIZATION_PROMPT

    def test_prompt_asks_for_key_points(self):
        assert "Decisions made" in SUMMARIZATION_PROMPT
        assert "Action items" in SUMMARIZATION_PROMPT


class TestFormatConversations:
    def test_formats_user_and_assistant(self):
        convos = [
            {"role": "user", "content": "Hello"},
            {"role": "assistant", "content": "Hi there!"},
        ]
        result = _format_conversations(convos)
        assert "User: Hello" in result
        assert "Mika: Hi there!" in result

    def test_preserves_order(self):
        convos = [
            {"role": "user", "content": "First"},
            {"role": "assistant", "content": "Second"},
            {"role": "user", "content": "Third"},
        ]
        result = _format_conversations(convos)
        lines = result.split("\n")
        assert len(lines) == 3
        assert "First" in lines[0]
        assert "Second" in lines[1]
        assert "Third" in lines[2]

    def test_empty_conversations(self):
        assert _format_conversations([]) == ""


class TestSummarizeText:
    def test_returns_empty_on_failure(self):
        with patch(
            "app.worker.tasks.maintenance.asyncio"
        ) as mock_asyncio:
            mock_loop = MagicMock()
            mock_loop.run_until_complete = MagicMock(
                side_effect=Exception("LLM failed")
            )
            mock_asyncio.get_event_loop.return_value = mock_loop

            from app.worker.tasks.maintenance import _summarize_text

            result = _summarize_text("some conversation")
            assert result == ""

    def test_calls_llm_via_event_loop(self):
        with patch(
            "app.worker.tasks.maintenance.asyncio"
        ) as mock_asyncio:
            mock_loop = MagicMock()
            mock_loop.run_until_complete = MagicMock(
                return_value="Summary of key points."
            )
            mock_asyncio.get_event_loop.return_value = mock_loop

            from app.worker.tasks.maintenance import _summarize_text

            result = _summarize_text("User: Hello\nMika: Hi!")
            assert result == "Summary of key points."
            mock_loop.run_until_complete.assert_called_once()


class TestGetUsersWithOldConversations:
    def test_returns_empty_on_failure(self):
        with patch(
            "app.common.db.sync_engine",
            side_effect=Exception("DB down"),
        ):
            from app.worker.tasks.maintenance import (
                _get_users_with_old_conversations,
            )

            result = _get_users_with_old_conversations()
            assert result == []
