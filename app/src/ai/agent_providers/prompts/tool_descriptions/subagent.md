Launch one or more subagents: separate agent conversations that work on their own tasks in parallel with this one.

Each call returns immediately. Every subagent's final answer is delivered back to this conversation as the tool result once it finishes, and the conversation continues automatically after all dispatched subagents have finished.

## Dispatch all independent subagents in ONE call

`tasks` is an array, and **one call can carry several subagents**. If you have two or more independent pieces of work, put **all** of them in the `tasks` array of a **single** `subagent` call — do not dispatch one, wait, and then dispatch the next, and do not split them across several `subagent` calls.

One call with N tasks launches all N subagents at once. Dispatching them one at a time is strictly worse: each dispatch ends your turn, so you cannot start the remaining work until the previous subagent has already finished and the conversation has resumed.

## When to use

- The work splits into genuinely independent workstreams that can run concurrently (for example researching two unrelated subsystems, or investigating two separate failures).

## When NOT to use

- The pieces depend on each other, or you need the result of one to decide the next step — do that work yourself, in order.
- The task is small enough to do directly. Do not use a subagent to avoid doing the task yourself.
- You only want to parallelize reads of the same files; use the search/read tools with batched calls instead.
