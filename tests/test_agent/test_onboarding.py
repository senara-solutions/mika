"""Tests for onboarding state machine and consent handling."""

import pytest

from app.agent.onboarding import (
    OnboardingState,
    can_transition,
    classify_consent_response,
    is_onboarding_active,
    next_state,
)


class TestOnboardingState:
    def test_all_states_defined(self):
        states = list(OnboardingState)
        assert len(states) == 6
        assert OnboardingState.AWAITING_CONSENT in states
        assert OnboardingState.COMPLETED in states

    def test_state_values(self):
        assert OnboardingState.AWAITING_CONSENT == "awaiting_consent"
        assert OnboardingState.COLLECTING_BASICS == "collecting_basics"
        assert OnboardingState.EXPLORING_PAIN == "exploring_pain"
        assert OnboardingState.IDENTIFYING_STUCK_TASK == "identifying_stuck_task"
        assert OnboardingState.DELIVERING_WOW == "delivering_wow"
        assert OnboardingState.COMPLETED == "completed"


class TestClassifyConsentResponse:
    def test_accepts_sounds_good(self):
        assert classify_consent_response("Sounds good") == "accepted"

    def test_accepts_yes(self):
        assert classify_consent_response("yes") == "accepted"

    def test_accepts_sure(self):
        assert classify_consent_response("Sure!") == "accepted"

    def test_accepts_lets_go(self):
        assert classify_consent_response("Let's go") == "accepted"

    def test_accepts_ok(self):
        assert classify_consent_response("OK") == "accepted"

    def test_accepts_case_insensitive(self):
        assert classify_consent_response("SOUNDS GOOD") == "accepted"

    def test_more_info_tell_me_more(self):
        assert classify_consent_response("Tell me more") == "more_info"

    def test_more_info_privacy(self):
        assert classify_consent_response("What about privacy?") == "more_info"

    def test_more_info_what_data(self):
        assert classify_consent_response("What data do you store?") == "more_info"

    def test_unclear_random_text(self):
        assert classify_consent_response("I like cats") == "unclear"

    def test_unclear_empty(self):
        assert classify_consent_response("") == "unclear"

    def test_unclear_question(self):
        assert classify_consent_response("Who are you?") == "unclear"


class TestTransitions:
    def test_valid_forward_transitions(self):
        assert can_transition("awaiting_consent", "collecting_basics")
        assert can_transition("collecting_basics", "exploring_pain")
        assert can_transition("exploring_pain", "identifying_stuck_task")
        assert can_transition("identifying_stuck_task", "delivering_wow")
        assert can_transition("delivering_wow", "completed")

    def test_cannot_skip_states(self):
        assert not can_transition("awaiting_consent", "exploring_pain")
        assert not can_transition("collecting_basics", "completed")

    def test_cannot_go_backwards(self):
        assert not can_transition("completed", "awaiting_consent")
        assert not can_transition("exploring_pain", "collecting_basics")

    def test_completed_has_no_transitions(self):
        assert not can_transition("completed", "completed")
        assert not can_transition("completed", "awaiting_consent")

    def test_invalid_state_returns_false(self):
        assert not can_transition("nonexistent", "collecting_basics")
        assert not can_transition("awaiting_consent", "nonexistent")


class TestNextState:
    def test_sequential_progression(self):
        assert next_state("awaiting_consent") == "collecting_basics"
        assert next_state("collecting_basics") == "exploring_pain"
        assert next_state("exploring_pain") == "identifying_stuck_task"
        assert next_state("identifying_stuck_task") == "delivering_wow"
        assert next_state("delivering_wow") == "completed"

    def test_completed_returns_none(self):
        assert next_state("completed") is None

    def test_invalid_state_returns_none(self):
        assert next_state("nonexistent") is None


class TestIsOnboardingActive:
    def test_active_states(self):
        assert is_onboarding_active("awaiting_consent")
        assert is_onboarding_active("collecting_basics")
        assert is_onboarding_active("exploring_pain")
        assert is_onboarding_active("identifying_stuck_task")
        assert is_onboarding_active("delivering_wow")

    def test_completed_not_active(self):
        assert not is_onboarding_active("completed")

    def test_none_not_active(self):
        assert not is_onboarding_active(None)


class TestConsentHandling:
    """Tests for the consent flow in invoke_agent."""

    @pytest.mark.asyncio
    async def test_consent_accepted_advances_state(self):
        from unittest.mock import AsyncMock, patch

        from app.agent.graph import _handle_consent

        with (
            patch(
                "app.agent.graph._update_onboarding_state",
                new_callable=AsyncMock,
            ) as mock_update,
            patch(
                "app.agent.graph._update_consent",
                new_callable=AsyncMock,
            ),
            patch(
                "app.agent.graph._save_conversation",
                new_callable=AsyncMock,
            ),
        ):
            result = await _handle_consent(
                "user-123", "Sounds good", "telegram"
            )
            mock_update.assert_called_once_with(
                "user-123", OnboardingState.COLLECTING_BASICS
            )
            assert "What should I call you" in result

    @pytest.mark.asyncio
    async def test_consent_more_info_stays_in_state(self):
        from unittest.mock import AsyncMock, patch

        from app.agent.graph import _handle_consent

        with (
            patch(
                "app.agent.graph._update_onboarding_state",
                new_callable=AsyncMock,
            ) as mock_update,
            patch(
                "app.agent.graph._save_conversation",
                new_callable=AsyncMock,
            ),
        ):
            result = await _handle_consent(
                "user-123", "Tell me more", "telegram"
            )
            mock_update.assert_not_called()
            assert "What I store" in result

    @pytest.mark.asyncio
    async def test_consent_unclear_stays_in_state(self):
        from unittest.mock import AsyncMock, patch

        from app.agent.graph import _handle_consent

        with (
            patch(
                "app.agent.graph._update_onboarding_state",
                new_callable=AsyncMock,
            ) as mock_update,
            patch(
                "app.agent.graph._save_conversation",
                new_callable=AsyncMock,
            ),
        ):
            result = await _handle_consent(
                "user-123", "I like cats", "telegram"
            )
            mock_update.assert_not_called()
            assert "Sounds good" in result


class TestOnboardingAdvancement:
    """Tests for the advance marker detection."""

    @pytest.mark.asyncio
    async def test_advance_marker_detected(self):
        from unittest.mock import AsyncMock, patch

        from app.agent.graph import _check_onboarding_advance

        with patch(
            "app.agent.graph._update_onboarding_state",
            new_callable=AsyncMock,
        ) as mock_update:
            result = await _check_onboarding_advance(
                "user-123",
                "collecting_basics",
                "Great answers!\n[ONBOARDING_ADVANCE]",
            )
            mock_update.assert_called_once_with(
                "user-123", "exploring_pain"
            )
            assert "[ONBOARDING_ADVANCE]" not in result
            assert "Great answers!" in result

    @pytest.mark.asyncio
    async def test_no_marker_no_advance(self):
        from unittest.mock import AsyncMock, patch

        from app.agent.graph import _check_onboarding_advance

        with patch(
            "app.agent.graph._update_onboarding_state",
            new_callable=AsyncMock,
        ) as mock_update:
            result = await _check_onboarding_advance(
                "user-123",
                "collecting_basics",
                "Tell me more about yourself!",
            )
            mock_update.assert_not_called()
            assert result == "Tell me more about yourself!"

    @pytest.mark.asyncio
    async def test_advance_to_completed_marks_done(self):
        from unittest.mock import AsyncMock, patch

        from app.agent.graph import _check_onboarding_advance

        with (
            patch(
                "app.agent.graph._update_onboarding_state",
                new_callable=AsyncMock,
            ),
            patch(
                "app.agent.graph._mark_onboarding_completed",
                new_callable=AsyncMock,
            ) as mock_complete,
        ):
            await _check_onboarding_advance(
                "user-123",
                "delivering_wow",
                "Here's your draft!\n[ONBOARDING_ADVANCE]",
            )
            mock_complete.assert_called_once_with("user-123")
