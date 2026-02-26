#!/bin/sh
# Web search handler stub.
# Replace this with a real search API call (e.g., SerpAPI, Brave Search, etc.)
#
# Input: JSON on stdin with "query" field
# Output: search results as text on stdout

INPUT=$(cat)
QUERY=$(echo "$INPUT" | grep -o '"query":"[^"]*"' | head -1 | cut -d'"' -f4)

echo "TODO: Implement web search for query: $QUERY"
echo "Replace this handler with a real search API integration."
