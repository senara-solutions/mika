"""Neo4j node and relationship type definitions."""

from enum import StrEnum


class NodeLabel(StrEnum):
    USER = "User"
    PERSON = "Person"
    COMMITMENT = "Commitment"
    TOPIC = "Topic"
    PATTERN = "Pattern"
    PREFERENCE = "Preference"
    FACT = "Fact"


class RelationType(StrEnum):
    HAS_MEMORY = "HAS_MEMORY"
    KNOWS = "KNOWS"
    COMMITTED_TO = "COMMITTED_TO"
    INVOLVES = "INVOLVES"
    INTERESTED_IN = "INTERESTED_IN"
    EXHIBITS = "EXHIBITS"
    PREFERS = "PREFERS"


# Merge keys for each node type (used for idempotent MERGE operations)
MERGE_KEYS = {
    NodeLabel.USER: ["user_id"],
    NodeLabel.PERSON: ["user_id", "canonical_name"],
    NodeLabel.COMMITMENT: ["user_id", "description_hash"],
    NodeLabel.TOPIC: ["user_id", "name"],
    NodeLabel.PATTERN: ["user_id", "type", "description_hash"],
    NodeLabel.PREFERENCE: ["user_id", "category"],
    NodeLabel.FACT: ["user_id", "key"],
}
