"""Tests for the LangGraph agent."""

from unittest.mock import patch

import pytest
from langchain_core.messages import AIMessage, HumanMessage

from app.agent.graph import _extract_response, build_graph
from app.agent.nodes.decide_action import decide_action
from app.agent.state import MikaState


class TestBuildGraph:
    def test_graph_builds_successfully(self):
        graph = build_graph()
        assert graph is not None

    def test_graph_compiles(self):
        graph = build_graph()
        compiled = graph.compile()
        assert compiled is not None


class TestDecideAction:
    def test_no_tool_calls_returns_respond(self):
        state: MikaState = {
            "user_id": "test",
            "messages": [AIMessage(content="Hello!")],
            "memory_context": "",
            "pending_actions": [],
            "personality_context": "",
            "step_count": 1,
            "error": None,
            "onboarding_state": None,
        }
        assert decide_action(state) == "respond"

    def test_tool_calls_returns_execute(self):
        msg = AIMessage(content="")
        msg.tool_calls = [
            {"name": "web_search", "args": {"query": "test"}, "id": "1"}
        ]
        state: MikaState = {
            "user_id": "test",
            "messages": [msg],
            "memory_context": "",
            "pending_actions": [],
            "personality_context": "",
            "step_count": 1,
            "error": None,
            "onboarding_state": None,
        }
        assert decide_action(state) == "execute_action"

    def test_step_limit_forces_respond(self):
        msg = AIMessage(content="")
        msg.tool_calls = [
            {"name": "web_search", "args": {"query": "test"}, "id": "1"}
        ]
        state: MikaState = {
            "user_id": "test",
            "messages": [msg],
            "memory_context": "",
            "pending_actions": [],
            "personality_context": "",
            "step_count": 10,
            "error": None,
            "onboarding_state": None,
        }
        assert decide_action(state) == "respond"

    def test_empty_messages_returns_respond(self):
        state: MikaState = {
            "user_id": "test",
            "messages": [],
            "memory_context": "",
            "pending_actions": [],
            "personality_context": "",
            "step_count": 0,
            "error": None,
            "onboarding_state": None,
        }
        assert decide_action(state) == "respond"


class TestExtractResponse:
    def test_extracts_last_ai_message(self):
        result = {
            "messages": [
                HumanMessage(content="Hello"),
                AIMessage(content="Hi there!"),
            ]
        }
        assert _extract_response(result) == "Hi there!"

    def test_empty_messages_returns_fallback(self):
        result = {"messages": []}
        assert "not sure" in _extract_response(result).lower()

    def test_no_ai_messages_returns_fallback(self):
        result = {"messages": [HumanMessage(content="Hello")]}
        assert "not sure" in _extract_response(result).lower()


class TestRetrieveMemory:
    @pytest.mark.asyncio
    async def test_returns_empty_on_failure(self):
        from app.agent.nodes.retrieve_memory import retrieve_memory

        state: MikaState = {
            "user_id": "test-id",
            "messages": [],
            "memory_context": "",
            "pending_actions": [],
            "personality_context": "",
            "step_count": 0,
            "error": None,
            "onboarding_state": None,
        }
        # Should not crash even without Neo4j
        with patch(
            "app.memory.repository.get_user_context",
            side_effect=Exception("Neo4j down"),
        ):
            result = await retrieve_memory(state)
            assert result["memory_context"] == ""
