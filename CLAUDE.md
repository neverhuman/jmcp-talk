# Claude Coordination Rules

Claude participates in JMCP work as a reviewer or leased worker. Codex owns integration unless an `AGENT_CHAT.md` handoff grants Claude a specific path and authority.

## Default Mode

- Read-only review and planning only.
- Do not edit files, start long-running services, kill processes, or push remotes.
- Report findings with file paths, line references where useful, and explicit assumptions.

## Leased Edit Mode

Claude may edit only after `AGENT_CHAT.md` records:

- the claimed paths,
- the expected task outcome,
- the allowed command/test surface,
- the handoff timestamp and owner.

Claude must append proof and changed-path notes to `AGENT_CHAT.md` before handing work back.

## Port Safety

JMCP must not bind Jeryu protected or retired-sensitive ports: `2224`, `8787`, `8929`, `18787`, `18788`, or `19800`.

JMCP defaults:

- API bind: `127.0.0.1:18877`
- API URL: `http://127.0.0.1:18877`
- cockpit dev bind: `127.0.0.1:15873`

Jeryu is optional. Its absence is degraded state, not a startup failure.
