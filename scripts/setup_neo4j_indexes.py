"""Create Neo4j indexes for the Mika knowledge graph."""

import asyncio

from neo4j import AsyncGraphDatabase

from app.config import settings

INDEXES = [
    "CREATE INDEX user_id_idx IF NOT EXISTS FOR (u:User) ON (u.user_id)",
    (
        "CREATE INDEX person_merge_idx IF NOT EXISTS"
        " FOR (p:Person) ON (p.user_id, p.canonical_name)"
    ),
    (
        "CREATE INDEX commitment_merge_idx IF NOT EXISTS"
        " FOR (c:Commitment) ON (c.user_id, c.description_hash)"
    ),
    "CREATE INDEX fact_merge_idx IF NOT EXISTS FOR (f:Fact) ON (f.user_id, f.key)",
    (
        "CREATE INDEX topic_merge_idx IF NOT EXISTS"
        " FOR (t:Topic) ON (t.user_id, t.name)"
    ),
    (
        "CREATE INDEX preference_merge_idx IF NOT EXISTS"
        " FOR (p:Preference) ON (p.user_id, p.category)"
    ),
    (
        "CREATE INDEX pattern_merge_idx IF NOT EXISTS"
        " FOR (p:Pattern) ON (p.user_id, p.type, p.description_hash)"
    ),
]


async def setup_indexes() -> None:
    driver = AsyncGraphDatabase.driver(
        settings.neo4j_uri,
        auth=(settings.neo4j_user, settings.neo4j_password),
    )
    async with driver:
        async with driver.session() as session:
            for index_query in INDEXES:
                await session.run(index_query)
                print(f"OK: {index_query}")
    print("All indexes created successfully.")


if __name__ == "__main__":
    asyncio.run(setup_indexes())
