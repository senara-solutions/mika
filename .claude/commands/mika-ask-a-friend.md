---
name: mika-ask-a-friend
description: Package the current problem + CC's proposed solution into a peer-review brief
argument-hint: "[optional: specific angle you want scrutinized]"
---

Vincent wants to get a second opinion from a peer (another Claude instance) on what
we're currently working through. Your job is NOT to answer him — it's to write the
brief he'll hand to the friend.

Produce a single markdown block he can copy-paste, structured exactly like this:

# <Concrete, specific title for this brief>

A real H1 title at the top so Vincent can sort multiple briefs at a glance.
Name the actual thing under discussion — the file/feature/bug/decision — not
a generic phrase. Examples of good vs bad:
- Good: `# Should mika-qa block PRs when CI is still pending?`
- Good: `# Race condition in claude-pilot relay reconnect on worker restart`
- Bad: `# Problem`, `# Peer review request`, `# Architecture question`

## Problem
One paragraph. What are we actually trying to solve? Include the concrete symptom
or goal, not the abstract category. If there's relevant code, file paths, error
output, or constraints from the conversation, surface them here.

## Context
The non-obvious stuff the friend needs to evaluate our reasoning: architectural
decisions already made, constraints Vincent has stated (stack, principles, etc.),
things we've already ruled out and why.

## What I'm proposing
Your current best answer or direction, stated as a committed position — not
hedged options. If you've already started implementing, say what and where.
If you're torn between approaches, say which one you'd pick and why.

## Where I'm uncertain
The specific parts where a second pair of eyes would actually help. Be honest —
if you're guessing, say you're guessing. If there's a tradeoff you resolved by
gut, name it.

## Vincent's note
$ARGUMENTS

---

Rules:
- Write the brief from *your* perspective ("I'm proposing…"), not Vincent's.
- Pull concrete details from the conversation — file names, function names, error
  messages, the actual code under discussion. No vague summaries.
- Do NOT add a solution recommendation for the friend. The friend decides.
- Do NOT ask Vincent clarifying questions. Work with what's in context.
- If $ARGUMENTS is empty, omit the "Vincent's note" section entirely.
- Output only the brief, in a single fenced markdown block, ready to paste.
