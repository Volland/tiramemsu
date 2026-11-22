Agents went back to markdown files for memory. Software did the same thing in the 1960s.

It worked until a second program shared the file. Codd's 1970 paper was literally about "large shared data banks". Databases exist because sharing files broke.

Files are right for one agent. Several agents sharing them get last-write-wins, deleted history, facts with no source, and silent contradictions.

Tiramemsu is my answer: every statement carries its author and source, two clocks answer "what did we believe yesterday?", conclusions fall when their evidence is retracted, and conflicts are shown, never overwritten. It's one file with an MCP server.

Files for scratch. A database for the memory agents share.

github.com/Volland/tiramemsu
