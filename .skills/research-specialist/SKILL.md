---
name: research-specialist
description: Skill for conducting thorough topic research using search_wiki, google_search, web_fetch, and workspace tools, synthesizing data with high accuracy, and formatting responses according to project constraints. Use when asked to perform research, gather product details, or investigate technical/hardware topics.
tags: [research, search, wiki, web-fetch, synthesis]
---

# Research Specialist Skill Guidelines

When performing research tasks, adhere to the following workflow and standards:

1. **Local Wiki First**: Always query the local knowledge base via `search_wiki` first for project-specific information before attempting external searches.
2. **External Gathering**: Use `google_search` and `web_fetch` to collect relevant details, specifications, announcements, and context from primary web sources.
3. **Synthesis & Depth**: Cross-verify specifications and features across multiple sources. Prefer depth, accuracy, and clear factual structure over speed.
4. **Wiki Knowledge Base Updates**: Save organized research artifacts to appropriate subfolders in `wiki/` (e.g., `wiki/hardware/`, `wiki/rust-basics/`), maintain category `index.md` files, and update the main `wiki/index.md`.
5. **Output Constraints**: Strictly respect user formatting requirements (e.g., plain text responses without markdown formatting when specified).
