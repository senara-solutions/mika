"""Web search tool."""

from langchain_core.tools import tool

from app.common.logging import get_logger

logger = get_logger(__name__)


@tool
async def web_search(query: str) -> str:
    """Search the web for information.

    Args:
        query: Search query string
    """
    try:
        import httpx

        from app.config import settings

        if not settings.tavily_api_key:
            return (
                "Web search is not configured. "
                "Please set TAVILY_API_KEY in environment."
            )

        async with httpx.AsyncClient() as client:
            response = await client.post(
                "https://api.tavily.com/search",
                json={
                    "api_key": settings.tavily_api_key,
                    "query": query,
                    "max_results": 5,
                },
                timeout=15.0,
            )
            response.raise_for_status()
            data = response.json()

        results = []
        for r in data.get("results", []):
            results.append(f"**{r['title']}**\n{r['content'][:300]}")

        return "\n\n".join(results) if results else "No results found."

    except Exception as e:
        logger.exception("Web search failed")
        return f"Search failed: {e!s}"
