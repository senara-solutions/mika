"""Route decision: execute tool, respond, or stop."""

from app.agent.state import MikaState


def decide_action(state: MikaState) -> str:
    """Decide next step based on LLM response."""
    # Hard stop at 10 steps
    if state.get("step_count", 0) >= 10:
        return "respond"

    # Check if the last message has tool calls
    messages = state.get("messages", [])
    if not messages:
        return "respond"

    last_message = messages[-1]
    if hasattr(last_message, "tool_calls") and last_message.tool_calls:
        return "execute_action"

    return "respond"
