AI agents rediscovered the filesystem. We have been here before.

1960s: data lived in flat files. Fine, until a second program needed the same file. Writes collided, copies drifted.

1970: Codd published "A Relational Model of Data for Large Shared Data Banks". Note the word shared. Databases exist because sharing files broke.

2000s: "just store the JSON". Then we rebuilt transactions on top.

2025: agents keep their memory in markdown. CLAUDE.md, memory folders, wikis.

For one agent, that is the right call. No schema. grep and edit are native. Humans can read every diff.

Now add a second agent.

Two of them write the same file. Last write wins.
One deletes a line. The history is gone.
A fact has no source and no confidence. Who said it?
Two files quietly disagree. Nobody notices.
And nobody can answer: what did the planner know when it made that call?

Files did not fail. Sharing failed. Again.

So I am building Tiramemsu: an embedded memory database for agents that work together.

→ Every statement carries its author and source.
→ Two clocks: when a fact was true in the world, and when memory learned it. "What did we believe yesterday?" is one query.
→ Nothing is deleted. Retract and supersede keep the full history.
→ Derived conclusions link to their evidence. Retract the evidence and the conclusion falls with it.
→ Conflicts are shown with who asserted each value, never silently resolved.
→ Shared rules (Datalog, N3) are enforced at commit for every agent.

Practical: one file (SQLite or redb), Rust/Node/Python/WASM bindings, and an MCP server:

claude mcp add tiramemsu -- tiramemsu-mcp --db ./memory.db

Give the reviewer agent --read-only: it queries, never writes.

Keep files as each agent's scratchpad. Put the memory they share in a database that remembers who said what, and when.

github.com/Volland/tiramemsu
tiramemsu.com

#AIAgents #MultiAgent #AgentMemory #Databases #KnowledgeGraphs
