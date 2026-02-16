"""Neo4j user-scoped memory repository."""

import hashlib
from typing import Any

from app.common.logging import get_logger
from app.memory.driver import get_neo4j_driver
from app.memory.schema import NodeLabel

logger = get_logger(__name__)


def _hash(text: str) -> str:
    """Create a short hash for merge keys."""
    return hashlib.sha256(text.encode()).hexdigest()[:16]


async def ensure_user_node(user_id: str) -> None:
    """Create or update the User anchor node."""
    driver = get_neo4j_driver()
    async with driver.session() as session:
        await session.run(
            "MERGE (u:User {user_id: $user_id}) "
            "ON CREATE SET u.created_at = datetime()",
            user_id=user_id,
        )


async def get_user_context(user_id: str, limit: int = 30) -> str:
    """Retrieve formatted memory context for LLM injection."""
    driver = get_neo4j_driver()
    async with driver.session() as session:
        result = await session.run(
            """
            MATCH (u:User {user_id: $user_id})-[:HAS_MEMORY]->(n)
            RETURN labels(n) AS labels, properties(n) AS props
            ORDER BY n.last_mentioned DESC, n.timestamp DESC
            LIMIT $limit
            """,
            user_id=user_id,
            limit=limit,
        )
        records = await result.data()

    if not records:
        return ""

    sections: dict[str, list[str]] = {}
    for record in records:
        label = record["labels"][0] if record["labels"] else "Unknown"
        props = record["props"]
        # Remove internal fields
        props.pop("user_id", None)
        props.pop("description_hash", None)

        summary = _format_node(label, props)
        if summary:
            sections.setdefault(label, []).append(summary)

    parts = []
    for label, items in sections.items():
        parts.append(f"### {label}s")
        for item in items:
            parts.append(f"- {item}")

    return "\n".join(parts)


def _format_node(label: str, props: dict) -> str:
    """Format a node into a human-readable string."""
    if label == NodeLabel.PERSON:
        name = props.get("name", props.get("canonical_name", "Unknown"))
        rel = props.get("relationship", "")
        ctx = props.get("context", "")
        return f"{name}" + (f" ({rel})" if rel else "") + (
            f" - {ctx}" if ctx else ""
        )
    elif label == NodeLabel.COMMITMENT:
        desc = props.get("description", "")
        due = props.get("due_date", "")
        status = props.get("status", "pending")
        return f"{desc}" + (f" (due: {due})" if due else "") + (
            f" [{status}]" if status else ""
        )
    elif label == NodeLabel.FACT:
        key = props.get("key", "")
        value = props.get("value", "")
        return f"{key}: {value}"
    elif label == NodeLabel.PREFERENCE:
        cat = props.get("category", "")
        val = props.get("value", "")
        return f"{cat}: {val}"
    elif label == NodeLabel.TOPIC:
        return props.get("name", "")
    elif label == NodeLabel.PATTERN:
        return props.get("description", "")
    return str(props)


async def write_entities(
    user_id: str, entities: list[dict[str, Any]]
) -> None:
    """Batch write entities to Neo4j using MERGE."""
    driver = get_neo4j_driver()
    async with driver.session() as session:
        await ensure_user_node(user_id)

        for entity in entities:
            label = entity.get("label")
            props = entity.get("properties", {})
            props["user_id"] = user_id

            if label == NodeLabel.PERSON:
                props["canonical_name"] = props.get(
                    "canonical_name", props.get("name", "").lower()
                )
                await session.run(
                    """
                    MATCH (u:User {user_id: $user_id})
                    MERGE (p:Person {
                        user_id: $user_id,
                        canonical_name: $canonical_name
                    })
                    ON CREATE SET p += $props, p.created_at = datetime()
                    ON MATCH SET p += $props,
                        p.last_mentioned = datetime()
                    MERGE (u)-[:HAS_MEMORY]->(p)
                    MERGE (u)-[:KNOWS]->(p)
                    """,
                    user_id=user_id,
                    canonical_name=props["canonical_name"],
                    props=props,
                )

            elif label == NodeLabel.COMMITMENT:
                desc_hash = _hash(props.get("description", ""))
                props["description_hash"] = desc_hash
                await session.run(
                    """
                    MATCH (u:User {user_id: $user_id})
                    MERGE (c:Commitment {
                        user_id: $user_id,
                        description_hash: $desc_hash
                    })
                    ON CREATE SET c += $props,
                        c.created_at = datetime()
                    ON MATCH SET c += $props
                    MERGE (u)-[:HAS_MEMORY]->(c)
                    MERGE (u)-[:COMMITTED_TO]->(c)
                    """,
                    user_id=user_id,
                    desc_hash=desc_hash,
                    props=props,
                )

            elif label == NodeLabel.FACT:
                await session.run(
                    """
                    MATCH (u:User {user_id: $user_id})
                    MERGE (f:Fact {user_id: $user_id, key: $key})
                    ON CREATE SET f += $props, f.created_at = datetime()
                    ON MATCH SET f += $props,
                        f.timestamp = datetime()
                    MERGE (u)-[:HAS_MEMORY]->(f)
                    """,
                    user_id=user_id,
                    key=props.get("key", ""),
                    props=props,
                )

            elif label == NodeLabel.PREFERENCE:
                await session.run(
                    """
                    MATCH (u:User {user_id: $user_id})
                    MERGE (p:Preference {
                        user_id: $user_id,
                        category: $category
                    })
                    ON CREATE SET p += $props, p.created_at = datetime()
                    ON MATCH SET p += $props
                    MERGE (u)-[:HAS_MEMORY]->(p)
                    MERGE (u)-[:PREFERS]->(p)
                    """,
                    user_id=user_id,
                    category=props.get("category", ""),
                    props=props,
                )

            elif label == NodeLabel.TOPIC:
                await session.run(
                    """
                    MATCH (u:User {user_id: $user_id})
                    MERGE (t:Topic {user_id: $user_id, name: $name})
                    ON CREATE SET t += $props, t.created_at = datetime()
                    ON MATCH SET t += $props, t.recency = datetime()
                    MERGE (u)-[:HAS_MEMORY]->(t)
                    MERGE (u)-[:INTERESTED_IN]->(t)
                    """,
                    user_id=user_id,
                    name=props.get("name", ""),
                    props=props,
                )

            elif label == NodeLabel.PATTERN:
                desc_hash = _hash(props.get("description", ""))
                props["description_hash"] = desc_hash
                await session.run(
                    """
                    MATCH (u:User {user_id: $user_id})
                    MERGE (p:Pattern {
                        user_id: $user_id,
                        type: $type,
                        description_hash: $desc_hash
                    })
                    ON CREATE SET p += $props,
                        p.first_observed = datetime(),
                        p.occurrences = 1
                    ON MATCH SET p += $props,
                        p.occurrences = p.occurrences + 1
                    MERGE (u)-[:HAS_MEMORY]->(p)
                    MERGE (u)-[:EXHIBITS]->(p)
                    """,
                    user_id=user_id,
                    type=props.get("type", ""),
                    desc_hash=desc_hash,
                    props=props,
                )


async def delete_user_memory(user_id: str) -> int:
    """Delete all memory nodes and relationships for a user (GDPR)."""
    driver = get_neo4j_driver()
    async with driver.session() as session:
        result = await session.run(
            """
            MATCH (u:User {user_id: $user_id})
            OPTIONAL MATCH (u)-[*]->(n)
            WITH u, collect(DISTINCT n) AS nodes
            FOREACH (n IN nodes | DETACH DELETE n)
            DETACH DELETE u
            RETURN size(nodes) + 1 AS deleted_count
            """,
            user_id=user_id,
        )
        record = await result.single()
        return record["deleted_count"] if record else 0


async def update_commitment_status(
    user_id: str,
    description: str,
    status: str,
) -> None:
    """Update a commitment's status (e.g., to 'completed')."""
    desc_hash = _hash(description)
    driver = get_neo4j_driver()
    async with driver.session() as session:
        await session.run(
            """
            MATCH (c:Commitment {
                user_id: $user_id,
                description_hash: $desc_hash
            })
            SET c.status = $status,
                c.completed_at = CASE WHEN $status = 'completed'
                    THEN datetime() ELSE c.completed_at END
            """,
            user_id=user_id,
            desc_hash=desc_hash,
            status=status,
        )


async def get_pending_commitments(
    user_id: str,
) -> list[dict]:
    """Get all pending commitments for a user."""
    driver = get_neo4j_driver()
    async with driver.session() as session:
        result = await session.run(
            """
            MATCH (u:User {user_id: $user_id})
                -[:COMMITTED_TO]->(c:Commitment)
            WHERE c.status = 'pending' OR c.status IS NULL
            RETURN properties(c) AS props
            ORDER BY c.due_date ASC
            """,
            user_id=user_id,
        )
        return await result.data()
