"""Tests for entity extraction."""

import json
from unittest.mock import AsyncMock, MagicMock, patch

import pytest
from langchain_core.messages import AIMessage


class TestExtractEntities:
    @pytest.mark.asyncio
    async def test_empty_input_returns_empty(self):
        from app.memory.extractor import extract_entities

        result = await extract_entities("")
        assert result == []

    @pytest.mark.asyncio
    async def test_short_input_returns_empty(self):
        from app.memory.extractor import extract_entities

        result = await extract_entities("hi")
        assert result == []

    @pytest.mark.asyncio
    async def test_successful_extraction(self):
        from app.memory.extractor import extract_entities

        entities_json = json.dumps([
            {
                "label": "Person",
                "properties": {
                    "name": "Sarah Chen",
                    "canonical_name": "sarah chen",
                    "relationship": "head of partnerships",
                },
            },
            {
                "label": "Commitment",
                "properties": {
                    "description": "Send proposal by Friday",
                    "status": "pending",
                },
            },
        ])

        mock_llm = MagicMock()
        mock_llm.ainvoke = AsyncMock(
            return_value=AIMessage(content=entities_json)
        )

        with patch(
            "app.memory.extractor.get_sonnet", return_value=mock_llm
        ):
            result = await extract_entities(
                "I need to send Sarah Chen the proposal by Friday"
            )

        assert len(result) == 2
        assert result[0]["label"] == "Person"
        assert result[1]["label"] == "Commitment"

    @pytest.mark.asyncio
    async def test_invalid_json_returns_empty(self):
        from app.memory.extractor import extract_entities

        mock_llm = MagicMock()
        mock_llm.ainvoke = AsyncMock(
            return_value=AIMessage(content="not json at all")
        )

        with patch(
            "app.memory.extractor.get_sonnet", return_value=mock_llm
        ):
            result = await extract_entities(
                "This is a normal conversation"
            )

        assert result == []

    @pytest.mark.asyncio
    async def test_filters_invalid_labels(self):
        from app.memory.extractor import extract_entities

        entities_json = json.dumps([
            {
                "label": "InvalidLabel",
                "properties": {"foo": "bar"},
            },
            {
                "label": "Fact",
                "properties": {"key": "team_size", "value": "12"},
            },
        ])

        mock_llm = MagicMock()
        mock_llm.ainvoke = AsyncMock(
            return_value=AIMessage(content=entities_json)
        )

        with patch(
            "app.memory.extractor.get_sonnet", return_value=mock_llm
        ):
            result = await extract_entities(
                "Our team has 12 people"
            )

        assert len(result) == 1
        assert result[0]["label"] == "Fact"

    @pytest.mark.asyncio
    async def test_handles_markdown_code_blocks(self):
        from app.memory.extractor import extract_entities

        entities_json = (
            '```json\n[{"label": "Fact", '
            '"properties": {"key": "role", "value": "CEO"}}]\n```'
        )

        mock_llm = MagicMock()
        mock_llm.ainvoke = AsyncMock(
            return_value=AIMessage(content=entities_json)
        )

        with patch(
            "app.memory.extractor.get_sonnet", return_value=mock_llm
        ):
            result = await extract_entities("I'm the CEO of Acme")

        assert len(result) == 1
        assert result[0]["properties"]["value"] == "CEO"
