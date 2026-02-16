"""Execute tool calls from LLM response."""

import asyncio

from langchain_core.messages import ToolMessage

from app.agent.state import MikaState
from app.common.logging import get_logger

logger = get_logger(__name__)

TOOL_TIMEOUT = 30  # seconds


async def execute_action(state: MikaState) -> dict:
    """Execute pending tool calls and return results."""
    messages = state.get("messages", [])
    if not messages:
        return {"error": "No messages to process"}

    last_message = messages[-1]
    if not hasattr(last_message, "tool_calls") or not last_message.tool_calls:
        return {}

    from app.tools import get_tools_by_name

    tools_by_name = get_tools_by_name()
    tool_messages: list[ToolMessage] = []

    for tool_call in last_message.tool_calls:
        tool_name = tool_call["name"]
        tool_args = tool_call["args"]
        tool_id = tool_call["id"]

        tool = tools_by_name.get(tool_name)
        if tool is None:
            tool_messages.append(
                ToolMessage(
                    content=f"Tool '{tool_name}' not found",
                    tool_call_id=tool_id,
                )
            )
            continue

        try:
            result = await asyncio.wait_for(
                tool.ainvoke(tool_args),
                timeout=TOOL_TIMEOUT,
            )
            tool_messages.append(
                ToolMessage(
                    content=str(result),
                    tool_call_id=tool_id,
                )
            )
        except TimeoutError:
            logger.warning(
                f"Tool '{tool_name}' timed out",
                extra={"user_id": state.get("user_id")},
            )
            tool_messages.append(
                ToolMessage(
                    content=f"Tool '{tool_name}' timed out after {TOOL_TIMEOUT}s",
                    tool_call_id=tool_id,
                )
            )
        except Exception as e:
            logger.exception(
                f"Tool '{tool_name}' failed",
                extra={"user_id": state.get("user_id")},
            )
            tool_messages.append(
                ToolMessage(
                    content=f"Tool '{tool_name}' error: {e!s}",
                    tool_call_id=tool_id,
                )
            )

    return {"messages": tool_messages}
