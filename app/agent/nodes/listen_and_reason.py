"""Core reasoning node -- calls Claude with context."""

from langchain_core.messages import SystemMessage

from app.agent.prompts import build_system_prompt
from app.agent.state import MikaState
from app.common.llm import get_sonnet
from app.common.logging import get_logger

logger = get_logger(__name__)


async def listen_and_reason(state: MikaState) -> dict:
    """Build prompt, call Claude, return response."""
    system_prompt = build_system_prompt(
        memory_context=state.get("memory_context", ""),
        onboarding_state=state.get("onboarding_state"),
    )

    # Build messages with system prompt
    messages = [SystemMessage(content=system_prompt)] + list(
        state["messages"]
    )

    # Get tools for binding
    from app.tools import get_tools

    tools = get_tools()

    llm = get_sonnet()
    if tools:
        llm_with_tools = llm.bind_tools(tools)
    else:
        llm_with_tools = llm

    response = await llm_with_tools.ainvoke(messages)

    return {
        "messages": [response],
        "step_count": state.get("step_count", 0) + 1,
    }
