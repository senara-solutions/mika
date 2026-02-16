"""Memory viewer and conversation history routes."""

from fastapi import APIRouter, Query, Request
from fastapi.responses import HTMLResponse, RedirectResponse
from sqlalchemy import func, select

from app.common.db import async_session_factory
from app.common.logging import get_logger
from app.models.conversation import Conversation

logger = get_logger(__name__)

router = APIRouter(prefix="/dashboard", tags=["memory"])


@router.get("/memory", response_class=HTMLResponse)
async def memory_viewer(request: Request):
    """Display user's knowledge graph memory."""
    user = getattr(request.state, "user", None)
    if not user:
        return RedirectResponse("/login", status_code=303)

    # Fetch memory from Neo4j
    memory_nodes = await _get_user_memory_nodes(str(user.id))

    return request.app.state.templates.TemplateResponse(
        "dashboard/memory.html",
        {"request": request, "user": user, "nodes": memory_nodes},
    )


@router.post("/memory/delete/{node_type}/{node_key}")
async def delete_memory_node(
    request: Request,
    node_type: str,
    node_key: str,
):
    """Delete a specific memory node."""
    user = getattr(request.state, "user", None)
    if not user:
        return RedirectResponse("/login", status_code=303)

    await _delete_node(str(user.id), node_type, node_key)
    return RedirectResponse("/dashboard/memory?deleted=1", status_code=303)


@router.get("/conversations", response_class=HTMLResponse)
async def conversation_history(
    request: Request,
    page: int = Query(1, ge=1),
    search: str = Query(""),
):
    """Display paginated conversation history."""
    user = getattr(request.state, "user", None)
    if not user:
        return RedirectResponse("/login", status_code=303)

    page_size = 20
    offset = (page - 1) * page_size

    async with async_session_factory() as session:
        # Count total
        count_query = select(func.count()).select_from(Conversation).where(
            Conversation.user_id == user.id
        )
        if search:
            count_query = count_query.where(
                Conversation.content.ilike(f"%{search}%")
            )
        total = (await session.execute(count_query)).scalar() or 0

        # Fetch page
        query = (
            select(Conversation)
            .where(Conversation.user_id == user.id)
            .order_by(Conversation.created_at.desc())
            .offset(offset)
            .limit(page_size)
        )
        if search:
            query = query.where(
                Conversation.content.ilike(f"%{search}%")
            )
        result = await session.execute(query)
        conversations = result.scalars().all()

    total_pages = max(1, (total + page_size - 1) // page_size)

    return request.app.state.templates.TemplateResponse(
        "dashboard/conversations.html",
        {
            "request": request,
            "user": user,
            "conversations": conversations,
            "page": page,
            "total_pages": total_pages,
            "search": search,
            "total": total,
        },
    )


async def _get_user_memory_nodes(user_id: str) -> list[dict]:
    """Get all memory nodes for a user from Neo4j."""
    try:
        from app.memory.driver import get_neo4j_driver

        driver = get_neo4j_driver()
        async with driver.session() as session:
            result = await session.run(
                """
                MATCH (u:User {user_id: $user_id})-[:HAS_MEMORY]->(n)
                RETURN labels(n) AS labels, properties(n) AS props
                ORDER BY n.last_mentioned DESC, n.timestamp DESC
                """,
                user_id=user_id,
            )
            records = await result.data()

        nodes = []
        for record in records:
            label = record["labels"][0] if record["labels"] else "Unknown"
            props = record["props"]
            props.pop("user_id", None)
            props.pop("description_hash", None)
            nodes.append({"label": label, "properties": props})
        return nodes
    except Exception:
        logger.debug("Could not fetch memory nodes")
        return []


async def _delete_node(
    user_id: str, node_type: str, node_key: str
) -> None:
    """Delete a specific memory node from Neo4j."""
    try:
        from app.memory.driver import get_neo4j_driver

        driver = get_neo4j_driver()
        async with driver.session() as session:
            # Use node type to build appropriate match
            if node_type == "Person":
                await session.run(
                    """
                    MATCH (n:Person {user_id: $user_id, canonical_name: $key})
                    DETACH DELETE n
                    """,
                    user_id=user_id,
                    key=node_key,
                )
            elif node_type == "Fact":
                await session.run(
                    """
                    MATCH (n:Fact {user_id: $user_id, key: $key})
                    DETACH DELETE n
                    """,
                    user_id=user_id,
                    key=node_key,
                )
            elif node_type == "Topic":
                await session.run(
                    """
                    MATCH (n:Topic {user_id: $user_id, name: $key})
                    DETACH DELETE n
                    """,
                    user_id=user_id,
                    key=node_key,
                )
            elif node_type == "Preference":
                await session.run(
                    """
                    MATCH (n:Preference {user_id: $user_id, category: $key})
                    DETACH DELETE n
                    """,
                    user_id=user_id,
                    key=node_key,
                )
    except Exception:
        logger.exception("Failed to delete memory node")
