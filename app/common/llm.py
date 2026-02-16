"""ChatAnthropic factory for Sonnet and Opus models."""

from langchain_anthropic import ChatAnthropic

from app.config import settings


def get_sonnet() -> ChatAnthropic:
    return ChatAnthropic(
        model=settings.claude_sonnet_model,
        api_key=settings.anthropic_api_key,
        max_tokens=4096,
        temperature=0.7,
    )


def get_opus() -> ChatAnthropic:
    return ChatAnthropic(
        model=settings.claude_opus_model,
        api_key=settings.anthropic_api_key,
        max_tokens=4096,
        temperature=0.7,
    )
