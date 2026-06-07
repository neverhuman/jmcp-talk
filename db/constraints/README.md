# Constraint Boundary

This directory documents that `jmcp-talk` has no database constraints of its
own. Speech runtime constraints are enforced through local bind addresses,
redaction, retention settings, and receipts.

If ownership is moved here later, the constraint proof must cover foreign key
integrity, row level security or equivalent tenant isolation, and a generated
contract boundary so data access stays compartmentalized.
