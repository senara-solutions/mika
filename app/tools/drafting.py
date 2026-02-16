"""Document and email drafting tool."""

from langchain_core.tools import tool

from app.common.llm import get_sonnet


@tool
async def draft_content(
    content_type: str,
    recipient: str,
    topic: str,
    context: str = "",
    tone: str = "professional",
) -> str:
    """Draft an email, message, or document.

    Args:
        content_type: Type of content (email, message, document, outline)
        recipient: Who is this for
        topic: What is this about
        context: Additional context or instructions
        tone: Desired tone (professional, casual, formal)
    """
    prompt = (
        f"Draft a {content_type} to {recipient} about {topic}.\n"
        f"Tone: {tone}\n"
    )
    if context:
        prompt += f"Additional context: {context}\n"
    prompt += "\nProvide only the draft content, no meta-commentary."

    llm = get_sonnet()
    response = await llm.ainvoke(prompt)
    return response.content
