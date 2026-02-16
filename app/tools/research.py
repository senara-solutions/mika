"""Research and analysis tools."""

from langchain_core.tools import tool

from app.common.llm import get_sonnet


@tool
async def research_topic(
    topic: str,
    depth: str = "brief",
) -> str:
    """Research a topic and provide a summary.

    Args:
        topic: The topic to research
        depth: Level of detail (brief, detailed)
    """
    prompt = (
        f"Provide a {depth} research summary on: {topic}\n\n"
        "Include key points, relevant facts, and actionable insights."
    )

    llm = get_sonnet()
    response = await llm.ainvoke(prompt)
    return response.content
