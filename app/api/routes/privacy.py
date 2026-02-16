"""Privacy API routes: data export and deletion."""

import io
import json
import zipfile
from datetime import datetime

from fastapi import APIRouter, Request
from fastapi.responses import JSONResponse, RedirectResponse, StreamingResponse
from sqlalchemy import select

from app.common.db import async_session_factory
from app.common.logging import get_logger
from app.models.channel import UserChannel
from app.models.conversation import Conversation
from app.models.user import User

logger = get_logger(__name__)

router = APIRouter(prefix="/api/privacy", tags=["privacy"])


@router.post("/export")
async def export_data(request: Request):
    """Export all user data as a ZIP file."""
    user = getattr(request.state, "user", None)
    if not user:
        return JSONResponse({"error": "Unauthorized"}, status_code=401)

    user_id = str(user.id)

    # Gather Postgres data
    profile, channels, conversations = await _gather_postgres_data(user_id)

    # Gather Neo4j memory
    memory_nodes = await _gather_memory_data(user_id)

    # Build ZIP
    zip_buffer = io.BytesIO()
    with zipfile.ZipFile(zip_buffer, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr("profile.json", json.dumps(profile, indent=2, default=str))
        zf.writestr("channels.json", json.dumps(channels, indent=2, default=str))
        zf.writestr(
            "conversations.json",
            json.dumps(conversations, indent=2, default=str),
        )
        zf.writestr("memory.json", json.dumps(memory_nodes, indent=2, default=str))

    zip_buffer.seek(0)
    timestamp = datetime.now().strftime("%Y%m%d_%H%M%S")

    return StreamingResponse(
        zip_buffer,
        media_type="application/zip",
        headers={
            "Content-Disposition": f'attachment; filename="mika_export_{timestamp}.zip"'
        },
    )


@router.post("/delete")
async def delete_data(request: Request):
    """Permanently delete all user data."""
    user = getattr(request.state, "user", None)
    if not user:
        return JSONResponse({"error": "Unauthorized"}, status_code=401)

    user_id = str(user.id)

    # 1. Delete Neo4j subgraph
    await _delete_neo4j_data(user_id)

    # 2. Delete Postgres user (cascades to conversations, channels, consent)
    await _delete_postgres_user(user_id)

    # 3. Clear Redis cache
    await _clear_redis_cache(user_id)

    logger.info("User data deleted", extra={"user_id": user_id})

    return JSONResponse({"ok": True, "message": "All data deleted"})


@router.get("/export")
async def export_page(request: Request):
    """Dashboard page to trigger export."""
    user = getattr(request.state, "user", None)
    if not user:
        return RedirectResponse("/login", status_code=303)
    return request.app.state.templates.TemplateResponse(
        "dashboard/privacy.html",
        {"request": request, "user": user},
    )


async def _gather_postgres_data(
    user_id: str,
) -> tuple[dict, list[dict], list[dict]]:
    """Gather all Postgres data for a user."""
    async with async_session_factory() as session:
        # Profile
        result = await session.execute(
            select(User).where(User.id == user_id)
        )
        user = result.scalar_one()
        profile = {
            "id": str(user.id),
            "email": user.email,
            "timezone": user.timezone,
            "preferred_channel": user.preferred_channel,
            "onboarding_completed": user.onboarding_completed,
            "created_at": str(user.created_at),
            "last_active_at": str(user.last_active_at),
        }

        # Channels
        ch_result = await session.execute(
            select(UserChannel).where(UserChannel.user_id == user_id)
        )
        channels = [
            {
                "channel_type": ch.channel_type,
                "channel_user_id": ch.channel_user_id,
                "connected_at": str(ch.connected_at),
            }
            for ch in ch_result.scalars().all()
        ]

        # Conversations
        conv_result = await session.execute(
            select(Conversation)
            .where(Conversation.user_id == user_id)
            .order_by(Conversation.created_at)
        )
        conversations = [
            {
                "role": c.role,
                "content": c.content,
                "channel_type": c.channel_type,
                "created_at": str(c.created_at),
            }
            for c in conv_result.scalars().all()
        ]

    return profile, channels, conversations


async def _gather_memory_data(user_id: str) -> list[dict]:
    """Gather all Neo4j memory nodes for a user."""
    try:
        from app.memory.driver import get_neo4j_driver

        driver = get_neo4j_driver()
        async with driver.session() as session:
            result = await session.run(
                """
                MATCH (u:User {user_id: $user_id})-[:HAS_MEMORY]->(n)
                RETURN labels(n) AS labels, properties(n) AS props
                """,
                user_id=user_id,
            )
            records = await result.data()

        nodes = []
        for record in records:
            label = record["labels"][0] if record["labels"] else "Unknown"
            props = dict(record["props"])
            props.pop("description_hash", None)
            nodes.append({"type": label, "properties": props})
        return nodes
    except Exception:
        logger.debug("Could not fetch memory nodes for export")
        return []


async def _delete_neo4j_data(user_id: str) -> None:
    """Delete all Neo4j nodes for a user."""
    try:
        from app.memory.driver import get_neo4j_driver

        driver = get_neo4j_driver()
        async with driver.session() as session:
            # Delete all memory nodes connected to user
            await session.run(
                """
                MATCH (u:User {user_id: $user_id})-[:HAS_MEMORY]->(n)
                DETACH DELETE n
                """,
                user_id=user_id,
            )
            # Delete user node itself
            await session.run(
                """
                MATCH (u:User {user_id: $user_id})
                DETACH DELETE u
                """,
                user_id=user_id,
            )
    except Exception:
        logger.exception("Failed to delete Neo4j data")


async def _delete_postgres_user(user_id: str) -> None:
    """Delete user from Postgres (cascades to all related tables)."""
    async with async_session_factory() as session:
        result = await session.execute(
            select(User).where(User.id == user_id)
        )
        user = result.scalar_one_or_none()
        if user:
            await session.delete(user)
            await session.commit()


async def _clear_redis_cache(user_id: str) -> None:
    """Clear any Redis keys for this user."""
    try:
        import redis.asyncio as aioredis

        from app.config import settings

        r = aioredis.from_url(settings.redis_url)
        # Delete rate limiter keys
        keys = await r.keys(f"proactive:*:{user_id}")
        if keys:
            await r.delete(*keys)
        await r.aclose()
    except Exception:
        logger.debug("Could not clear Redis cache for user")
