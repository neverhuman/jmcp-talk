# JMCP Talk DB Boundary

`jmcp-talk` does not own durable product truth, approval ledgers, or policy
state. Those boundaries live in `jmcp-core`.

Use this directory only for negative ownership guidance, migration placeholders,
and constraint documentation that keeps agents from adding direct database
ownership to the speech runtime.
