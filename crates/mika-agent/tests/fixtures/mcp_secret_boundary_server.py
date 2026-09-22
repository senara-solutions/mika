#!/usr/bin/env python3
"""Serveur MCP stdio factice — frontière de secret (mika#2281, UI-3).

Ce fixture reproduit le **contrat** du serveur MCP « 1Password Environments »,
pas son implémentation : il *détient* une valeur de secret et ne la rend
**jamais** par le canal MCP. Seuls des noms de variables traversent.

Il expose deux outils, modelés sur ceux de l'éditeur :

- ``list_environments`` — rend des noms d'Environments et des noms de
  variables. Jamais de valeur.
- ``create_local_env_file`` — écrit un fichier ``.env`` à l'emplacement
  demandé (l'équivalent, pour un test, du montage FIFO en mémoire du vrai
  serveur) et rend le chemin plus la liste des noms de variables. Jamais de
  valeur.

C'est cette asymétrie qui rend le test mesurable : la sentinelle existe, elle
est écrite sur le disque, et si elle apparaît dans un canal durable c'est
qu'elle a traversé le canal *fichier* — jamais le canal MCP. Sans le contrôle
positif d'UI-3 point 4, un test tout-vert ici ne prouverait rien : il serait
indistinguable d'un test qui ne mesure rien.

Protocole : JSON-RPC 2.0 en lignes délimitées par ``\\n`` sur stdin/stdout,
la forme du transport stdio MCP. On répond ``initialize`` en **renvoyant la
version de protocole demandée par le client** plutôt qu'une version figée :
le fixture n'a pas d'opinion sur la négociation, et une version en dur
deviendrait fausse au prochain bump de rmcp.

Variables d'environnement lues (passées par ``McpServerConfig.env`` ; aucune
ne commence par ``MIKA_``, que ``connect_stdio`` refuse à dessein) :

- ``MCP_PROBE_SENTINEL``    — la valeur de secret détenue (obligatoire)
- ``MCP_PROBE_VAR_NAME``    — le nom de la variable qui la porte
                              (défaut ``DATABASE_URL``)
- ``MCP_PROBE_ENVIRONMENT`` — le nom de l'Environment de test
                              (défaut ``mika-test-2281``)
"""

import json
import os
import sys

SENTINEL = os.environ.get("MCP_PROBE_SENTINEL", "")
VAR_NAME = os.environ.get("MCP_PROBE_VAR_NAME", "DATABASE_URL")
ENVIRONMENT = os.environ.get("MCP_PROBE_ENVIRONMENT", "mika-test-2281")

# La seconde variable est publique à dessein : elle prouve que le serveur sait
# rendre des noms, et que taire la première n'est pas un mutisme général.
PUBLIC_VAR_NAME = "API_BASE_URL"
PUBLIC_VAR_VALUE = "https://example.invalid/v1"

TOOLS = [
    {
        "name": "list_environments",
        "description": (
            "List the Environments this vault exposes. Returns variable NAMES "
            "only; values are never returned to the client."
        ),
        "inputSchema": {"type": "object", "properties": {}},
    },
    {
        "name": "create_local_env_file",
        "description": (
            "Materialise an Environment as a local .env file. Returns the path "
            "and the variable NAMES written; values are never returned."
        ),
        "inputSchema": {
            "type": "object",
            "properties": {
                "environment": {"type": "string"},
                "path": {"type": "string"},
            },
            "required": ["path"],
        },
    },
]


def _text_result(payload):
    """Enveloppe un objet JSON en `CallToolResult` à un seul bloc texte."""
    return {
        "content": [{"type": "text", "text": json.dumps(payload, sort_keys=True)}],
        "isError": False,
    }


def _error_result(message):
    return {
        "content": [{"type": "text", "text": message}],
        "isError": True,
    }


def _call_tool(params):
    name = params.get("name", "")
    args = params.get("arguments") or {}

    if name == "list_environments":
        return _text_result(
            {
                "environments": [
                    {
                        "name": ENVIRONMENT,
                        # Des NOMS. C'est tout ce qui sort d'ici.
                        "variables": [VAR_NAME, PUBLIC_VAR_NAME],
                    }
                ]
            }
        )

    if name == "create_local_env_file":
        path = args.get("path")
        if not path:
            return _error_result("create_local_env_file requires 'path'")
        lines = [
            f"{VAR_NAME}={SENTINEL}",
            f"{PUBLIC_VAR_NAME}={PUBLIC_VAR_VALUE}",
            "",
        ]
        try:
            with open(path, "w", encoding="utf-8") as handle:
                handle.write("\n".join(lines))
            os.chmod(path, 0o600)
        except OSError as exc:  # pragma: no cover - chemin d'erreur du fixture
            return _error_result(f"failed to write env file: {exc}")
        return _text_result(
            {
                "mounted": True,
                "environment": args.get("environment", ENVIRONMENT),
                "path": path,
                # Encore des noms. La valeur n'est écrite que sur le disque.
                "variables": [VAR_NAME, PUBLIC_VAR_NAME],
            }
        )

    return _error_result(f"unknown tool: {name}")


def _handle(message):
    """Rend le `result` d'une requête, ou `None` pour une notification."""
    method = message.get("method", "")

    if method == "initialize":
        requested = (message.get("params") or {}).get("protocolVersion")
        return {
            "protocolVersion": requested or "2025-06-18",
            "capabilities": {"tools": {}},
            "serverInfo": {
                "name": "mika-secret-boundary-probe",
                "version": "0.1.0",
            },
        }

    if method == "tools/list":
        return {"tools": TOOLS}

    if method == "tools/call":
        return _call_tool(message.get("params") or {})

    if method == "ping":
        return {}

    return None


def main():
    for line in sys.stdin:
        line = line.strip()
        if not line:
            continue
        try:
            message = json.loads(line)
        except json.JSONDecodeError:
            continue

        # Pas d'`id` ⇒ notification (`notifications/initialized`, `cancelled`,
        # …) : on ne répond rien, comme le veut JSON-RPC.
        if "id" not in message:
            continue

        result = _handle(message)
        if result is None:
            response = {
                "jsonrpc": "2.0",
                "id": message["id"],
                "error": {
                    "code": -32601,
                    "message": f"method not found: {message.get('method', '')}",
                },
            }
        else:
            response = {"jsonrpc": "2.0", "id": message["id"], "result": result}

        sys.stdout.write(json.dumps(response) + "\n")
        sys.stdout.flush()


if __name__ == "__main__":
    main()
