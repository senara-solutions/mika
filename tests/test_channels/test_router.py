"""Tests for message router and message splitting."""

from app.channels.router import split_message


def test_split_message_short():
    """Short messages should not be split."""
    result = split_message("Hello world")
    assert result == ["Hello world"]


def test_split_message_at_paragraph_boundary():
    """Long messages should split at paragraph boundaries."""
    text = "First paragraph.\n\nSecond paragraph.\n\nThird paragraph."
    result = split_message(text, max_length=40)
    assert len(result) >= 2
    assert "First paragraph." in result[0]


def test_split_message_at_newline():
    """Falls back to newline split when no paragraph break."""
    text = "Line one\nLine two\nLine three\nLine four very long"
    result = split_message(text, max_length=25)
    assert len(result) >= 2


def test_split_message_at_space():
    """Falls back to space split when no newlines."""
    text = "word " * 20
    result = split_message(text.strip(), max_length=30)
    assert len(result) >= 2


def test_split_message_hard_cut():
    """Hard cut when no break points available."""
    text = "a" * 100
    result = split_message(text, max_length=30)
    assert len(result) >= 3
    assert all(len(chunk) <= 30 for chunk in result)


def test_split_message_exact_boundary():
    """Message exactly at max_length should not be split."""
    text = "a" * 4096
    result = split_message(text, max_length=4096)
    assert len(result) == 1
