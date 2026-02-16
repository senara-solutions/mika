"""Tests for memory repository formatting and hashing."""

from app.memory.repository import _format_node, _hash
from app.memory.schema import NodeLabel


class TestHash:
    def test_consistent_hashing(self):
        assert _hash("test") == _hash("test")

    def test_different_inputs(self):
        assert _hash("foo") != _hash("bar")

    def test_hash_length(self):
        assert len(_hash("anything")) == 16


class TestFormatNode:
    def test_format_person(self):
        result = _format_node(
            NodeLabel.PERSON,
            {
                "name": "Sarah Chen",
                "relationship": "head of partnerships",
                "context": "works at Acme Corp",
            },
        )
        assert "Sarah Chen" in result
        assert "partnerships" in result

    def test_format_commitment(self):
        result = _format_node(
            NodeLabel.COMMITMENT,
            {
                "description": "Send proposal to Sarah",
                "due_date": "2026-02-20",
                "status": "pending",
            },
        )
        assert "proposal" in result
        assert "2026-02-20" in result
        assert "pending" in result

    def test_format_fact(self):
        result = _format_node(
            NodeLabel.FACT,
            {"key": "team_size", "value": "12"},
        )
        assert "team_size: 12" == result

    def test_format_preference(self):
        result = _format_node(
            NodeLabel.PREFERENCE,
            {"category": "communication", "value": "bullet points"},
        )
        assert "communication" in result
        assert "bullet points" in result

    def test_format_topic(self):
        result = _format_node(
            NodeLabel.TOPIC,
            {"name": "Q2 hiring plan"},
        )
        assert "Q2 hiring plan" == result

    def test_format_pattern(self):
        result = _format_node(
            NodeLabel.PATTERN,
            {"description": "Procrastinates on finance tasks"},
        )
        assert "Procrastinates" in result
