"""Tool definitions for the Mika agent."""

from langchain_core.tools import BaseTool

from app.tools.drafting import draft_content
from app.tools.research import research_topic
from app.tools.search import web_search

ALL_TOOLS: list[BaseTool] = [draft_content, web_search, research_topic]


def get_tools() -> list[BaseTool]:
    return ALL_TOOLS


def get_tools_by_name() -> dict[str, BaseTool]:
    return {tool.name: tool for tool in ALL_TOOLS}
