"""Entity extraction from conversations using Claude."""

import json

from app.common.llm import get_sonnet
from app.common.logging import get_logger
from app.memory.schema import NodeLabel

logger = get_logger(__name__)

EXTRACTION_PROMPT = """Analyze this conversation and extract structured entities.

Return a JSON array of entities. Each entity should have:
- "label": one of "Person", "Commitment", "Fact", "Preference", "Topic", "Pattern"
- "properties": an object with relevant fields

Entity types and their properties:
- Person: name, canonical_name (lowercase), relationship, context, sentiment
- Commitment: description, due_date (if mentioned, ISO format), status ("pending"), priority
- Fact: key (short identifier), value (the fact), source ("conversation")
- Preference: category (e.g. "communication_style"), value, confidence ("high"/"medium"/"low")
- Topic: name, importance ("high"/"medium"/"low")
- Pattern: type (e.g. "work_habit"), description, confidence ("high"/"medium"/"low")

Rules:
- Only extract entities clearly stated or strongly implied
- For entity resolution: normalize names to canonical form (e.g. "Sarah" -> "sarah chen")
- For commitments: only extract if the user explicitly commits to doing something
- Return an empty array [] if nothing meaningful can be extracted
- Keep descriptions concise

Conversation:
{conversation_text}

Return ONLY valid JSON, no markdown formatting."""


async def extract_entities(
    conversation_text: str,
) -> list[dict]:
    """Extract entities from conversation text using Claude.

    Returns a list of entity dicts with 'label' and 'properties' keys.
    Returns empty list on any failure.
    """
    if not conversation_text or len(conversation_text.strip()) < 10:
        return []

    try:
        llm = get_sonnet()
        prompt = EXTRACTION_PROMPT.format(
            conversation_text=conversation_text
        )
        response = await llm.ainvoke(prompt)
        content = response.content.strip()

        # Handle markdown code blocks
        if content.startswith("```"):
            content = content.split("\n", 1)[1]
            content = content.rsplit("```", 1)[0]

        entities = json.loads(content)

        if not isinstance(entities, list):
            logger.warning("Extraction returned non-list")
            return []

        # Validate each entity
        valid = []
        valid_labels = {label.value for label in NodeLabel} - {
            NodeLabel.USER
        }
        for entity in entities:
            label = entity.get("label")
            props = entity.get("properties")
            if label in valid_labels and isinstance(props, dict):
                valid.append(entity)

        return valid

    except json.JSONDecodeError:
        logger.warning("Failed to parse extraction response as JSON")
        return []
    except Exception:
        logger.exception("Entity extraction failed")
        return []
