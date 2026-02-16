"""LangGraph agent graph definition and invocation."""

import uuid
from datetime import UTC, datetime

from langchain_core.messages import HumanMessage
from langgraph.checkpoint.memory import InMemorySaver
from langgraph.graph import END, START, StateGraph
from sqlalchemy import select

from app.agent.nodes.decide_action import decide_action
from app.agent.nodes.execute_action import execute_action
from app.agent.nodes.listen_and_reason import listen_and_reason
from app.agent.nodes.respond import respond
from app.agent.nodes.retrieve_memory import retrieve_memory
from app.agent.onboarding import (
    OnboardingState,
    classify_consent_response,
    is_onboarding_active,
    next_state,
)
from app.agent.prompts import PRIVACY_DETAILS
from app.agent.state import MikaState
from app.common.logging import get_logger
from app.models.conversation import Conversation
from app.models.user import User

logger = get_logger(__name__)

ADVANCE_MARKER = "[ONBOARDING_ADVANCE]"


def build_graph() -> StateGraph:
    """Build and compile the Mika agent graph."""
    graph = StateGraph(MikaState)

    graph.add_node("retrieve_memory", retrieve_memory)
    graph.add_node("listen_and_reason", listen_and_reason)
    graph.add_node("execute_action", execute_action)
    graph.add_node("respond", respond)

    # Flow: START -> retrieve_memory -> listen_and_reason -> decide
    graph.add_edge(START, "retrieve_memory")
    graph.add_edge("retrieve_memory", "listen_and_reason")

    # Conditional: decide_action routes to execute_action or respond
    graph.add_conditional_edges(
        "listen_and_reason",
        decide_action,
        {
            "execute_action": "execute_action",
            "respond": "respond",
        },
    )

    # After tool execution, go back to reasoning
    graph.add_edge("execute_action", "listen_and_reason")

    # respond is terminal
    graph.add_edge("respond", END)

    return graph


# Compiled graph with in-memory checkpointer (dev)
_checkpointer = InMemorySaver()
_compiled_graph = build_graph().compile(
    checkpointer=_checkpointer,
)


async def invoke_agent(
    user_id: str,
    message_text: str,
    channel_type: str,
) -> str:
    """Invoke the Mika agent. Returns response text."""
    logger.info(
        "Agent invoked",
        extra={"user_id": user_id, "channel_type": channel_type},
    )

    # Get user's onboarding state
    onboarding_state = await _get_onboarding_state(user_id)

    # Handle consent phase before invoking the full agent
    if onboarding_state == OnboardingState.AWAITING_CONSENT:
        return await _handle_consent(user_id, message_text, channel_type)

    # Load conversation history from Postgres
    history_messages = await _load_conversation_history(user_id)

    # Build initial state
    all_messages = history_messages + [HumanMessage(content=message_text)]
    initial_state: MikaState = {
        "user_id": user_id,
        "messages": all_messages,
        "memory_context": "",
        "pending_actions": [],
        "personality_context": "",
        "step_count": 0,
        "error": None,
        "onboarding_state": onboarding_state,
    }

    config = {
        "configurable": {"thread_id": f"user-{user_id}"},
        "recursion_limit": 10,
    }

    try:
        result = await _compiled_graph.ainvoke(initial_state, config)
    except Exception:
        logger.exception("Agent graph failed", extra={"user_id": user_id})
        return (
            "I'm having trouble right now. I'll follow up shortly."
        )

    # Extract response text
    response_text = _extract_response(result)

    # Check for onboarding state advancement
    if is_onboarding_active(onboarding_state):
        response_text = await _check_onboarding_advance(
            user_id, onboarding_state, response_text
        )

    # Save conversation to Postgres
    await _save_conversation(
        user_id, channel_type, message_text, response_text
    )

    # Trigger async memory extraction
    try:
        from app.worker.tasks.memory_extraction import (
            extract_and_store_memory,
        )

        extract_and_store_memory.delay(user_id, message_text)
    except Exception:
        logger.debug("Memory extraction not available yet")

    return response_text


async def _handle_consent(
    user_id: str,
    message_text: str,
    channel_type: str,
) -> str:
    """Handle the consent phase of onboarding."""
    classification = classify_consent_response(message_text)

    if classification == "accepted":
        await _update_onboarding_state(
            user_id, OnboardingState.COLLECTING_BASICS
        )
        await _update_consent(user_id, llm=True, memory=True)
        response = (
            "Great, let's get started!\n\n"
            "I'd love to learn a bit about you so I can be most helpful. "
            "What should I call you?"
        )
    elif classification == "more_info":
        response = PRIVACY_DETAILS
    else:
        response = (
            "No worries! Just reply **Sounds good** when you're ready "
            "to get started, or **Tell me more** if you'd like to know "
            "more about how your data is handled."
        )

    await _save_conversation(
        user_id, channel_type, message_text, response
    )
    return response


async def _check_onboarding_advance(
    user_id: str,
    current_state: str,
    response_text: str,
) -> str:
    """Check if the agent wants to advance the onboarding state."""
    if ADVANCE_MARKER in response_text:
        new_state = next_state(current_state)
        if new_state:
            await _update_onboarding_state(user_id, new_state)
            logger.info(
                "Onboarding advanced",
                extra={
                    "user_id": user_id,
                    "from": current_state,
                    "to": new_state,
                },
            )
            if new_state == OnboardingState.COMPLETED:
                await _mark_onboarding_completed(user_id)
        # Strip the marker from the response
        response_text = response_text.replace(ADVANCE_MARKER, "").strip()

    return response_text


def _extract_response(result: dict) -> str:
    """Extract the final text response from graph result."""
    messages = result.get("messages", [])
    if not messages:
        return "I'm not sure how to respond to that."

    # Find the last AI message with content
    for msg in reversed(messages):
        content = getattr(msg, "content", "")
        if content and getattr(msg, "type", "") == "ai":
            return content

    return "I'm not sure how to respond to that."


async def _load_conversation_history(
    user_id: str, limit: int = 20
) -> list[HumanMessage]:
    """Load recent conversation history from Postgres."""
    try:
        from app.common.db import async_session_factory

        async with async_session_factory() as session:
            result = await session.execute(
                select(Conversation)
                .where(Conversation.user_id == uuid.UUID(user_id))
                .order_by(Conversation.created_at.desc())
                .limit(limit)
            )
            conversations = list(reversed(result.scalars().all()))

            from langchain_core.messages import AIMessage

            messages = []
            for conv in conversations:
                if conv.role == "user":
                    messages.append(HumanMessage(content=conv.content))
                elif conv.role == "assistant":
                    messages.append(AIMessage(content=conv.content))
            return messages
    except Exception:
        logger.debug("Could not load conversation history")
        return []


async def _get_onboarding_state(user_id: str) -> str | None:
    """Get user's onboarding state from Postgres."""
    try:
        from app.common.db import async_session_factory

        async with async_session_factory() as session:
            result = await session.execute(
                select(User.onboarding_state).where(
                    User.id == uuid.UUID(user_id)
                )
            )
            state = result.scalar_one_or_none()
            return state
    except Exception:
        return None


async def _update_onboarding_state(
    user_id: str, new_state: str
) -> None:
    """Update user's onboarding state in Postgres."""
    try:
        from app.common.db import async_session_factory

        async with async_session_factory() as session:
            result = await session.execute(
                select(User).where(User.id == uuid.UUID(user_id))
            )
            user = result.scalar_one_or_none()
            if user:
                user.onboarding_state = new_state
                await session.commit()
    except Exception:
        logger.exception("Failed to update onboarding state")


async def _mark_onboarding_completed(user_id: str) -> None:
    """Mark user's onboarding as completed."""
    try:
        from app.common.db import async_session_factory

        async with async_session_factory() as session:
            result = await session.execute(
                select(User).where(User.id == uuid.UUID(user_id))
            )
            user = result.scalar_one_or_none()
            if user:
                user.onboarding_completed = True
                user.onboarding_state = OnboardingState.COMPLETED
                await session.commit()
    except Exception:
        logger.exception("Failed to mark onboarding completed")


async def _update_consent(
    user_id: str, *, llm: bool, memory: bool
) -> None:
    """Update user consent flags."""
    try:
        from app.common.db import async_session_factory
        from app.models.consent import UserConsent

        async with async_session_factory() as session:
            result = await session.execute(
                select(UserConsent).where(
                    UserConsent.user_id == uuid.UUID(user_id)
                )
            )
            consent = result.scalar_one_or_none()
            if consent:
                consent.llm_processing = llm
                consent.memory_storage = memory
                consent.consented_at = datetime.now(UTC)
                await session.commit()
    except Exception:
        logger.exception("Failed to update consent")


async def _save_conversation(
    user_id: str,
    channel_type: str,
    user_message: str,
    assistant_response: str,
) -> None:
    """Save conversation turn to Postgres."""
    try:
        from app.common.db import async_session_factory

        async with async_session_factory() as session:
            uid = uuid.UUID(user_id)
            session.add(
                Conversation(
                    user_id=uid,
                    channel_type=channel_type,
                    role="user",
                    content=user_message,
                    created_at=datetime.now(UTC),
                )
            )
            session.add(
                Conversation(
                    user_id=uid,
                    channel_type=channel_type,
                    role="assistant",
                    content=assistant_response,
                    created_at=datetime.now(UTC),
                )
            )
            await session.commit()
    except Exception:
        logger.debug("Could not save conversation")
