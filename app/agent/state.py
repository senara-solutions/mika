"""Mika agent state definition."""

from typing import Annotated

from langchain_core.messages import BaseMessage
from langgraph.graph.message import add_messages
from typing_extensions import TypedDict


class MikaState(TypedDict):
    user_id: str
    messages: Annotated[list[BaseMessage], add_messages]
    memory_context: str
    pending_actions: list
    personality_context: str
    step_count: int
    error: str | None
    onboarding_state: str | None
