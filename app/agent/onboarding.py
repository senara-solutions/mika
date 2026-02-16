"""Onboarding state machine for new users.

States: awaiting_consent -> collecting_basics -> exploring_pain
        -> identifying_stuck_task -> delivering_wow -> completed
"""

from enum import StrEnum

from app.common.logging import get_logger

logger = get_logger(__name__)


class OnboardingState(StrEnum):
    AWAITING_CONSENT = "awaiting_consent"
    COLLECTING_BASICS = "collecting_basics"
    EXPLORING_PAIN = "exploring_pain"
    IDENTIFYING_STUCK_TASK = "identifying_stuck_task"
    DELIVERING_WOW = "delivering_wow"
    COMPLETED = "completed"


# Valid transitions: current_state -> set of allowed next states
TRANSITIONS = {
    OnboardingState.AWAITING_CONSENT: {OnboardingState.COLLECTING_BASICS},
    OnboardingState.COLLECTING_BASICS: {OnboardingState.EXPLORING_PAIN},
    OnboardingState.EXPLORING_PAIN: {OnboardingState.IDENTIFYING_STUCK_TASK},
    OnboardingState.IDENTIFYING_STUCK_TASK: {OnboardingState.DELIVERING_WOW},
    OnboardingState.DELIVERING_WOW: {OnboardingState.COMPLETED},
    OnboardingState.COMPLETED: set(),
}


def classify_consent_response(text: str) -> str:
    """Classify user response during consent phase.

    Returns: 'accepted', 'more_info', or 'unclear'.
    """
    lower = text.strip().lower()

    accept_phrases = [
        "sounds good",
        "yes",
        "sure",
        "ok",
        "okay",
        "let's go",
        "lets go",
        "i agree",
        "i consent",
        "go ahead",
        "continue",
        "start",
        "yep",
        "yeah",
        "absolutely",
        "of course",
        "fine",
        "alright",
        "deal",
        "cool",
    ]
    more_info_phrases = [
        "tell me more",
        "more info",
        "details",
        "how does it work",
        "what data",
        "privacy",
        "explain",
        "what do you store",
        "how do you use",
    ]

    for phrase in accept_phrases:
        if phrase in lower:
            return "accepted"

    for phrase in more_info_phrases:
        if phrase in lower:
            return "more_info"

    return "unclear"


def can_transition(
    current: str, target: str
) -> bool:
    """Check if a state transition is valid."""
    try:
        current_state = OnboardingState(current)
        target_state = OnboardingState(target)
    except ValueError:
        return False
    return target_state in TRANSITIONS.get(current_state, set())


def next_state(current: str) -> str | None:
    """Get the next sequential onboarding state, or None if completed."""
    try:
        current_state = OnboardingState(current)
    except ValueError:
        return None
    allowed = TRANSITIONS.get(current_state, set())
    if not allowed:
        return None
    return next(iter(allowed))


def is_onboarding_active(state: str | None) -> bool:
    """Check if the user is still in onboarding."""
    if state is None:
        return False
    return state != OnboardingState.COMPLETED
