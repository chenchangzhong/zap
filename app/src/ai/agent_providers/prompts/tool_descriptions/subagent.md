Launch one or more subagents: separate agent conversations that each work on their own task in parallel with this one.

Each call returns immediately; every subagent's final answer is delivered back to this conversation as the tool result, and this conversation continues automatically once all of them have finished.

## When to use

**You decide — the user does not need to ask for a subagent.** If the work splits into independent workstreams, dispatching beats doing them one after another.

The test: can you name **two or more workstreams that do not need each other's results**? For example researching two unrelated subsystems, or investigating two separate failures.

When the test passes and the pieces are not trivial, **prefer dispatching over working serially**.

## When NOT to use

- The pieces depend on each other, or you need the result of one to decide the next step — do that work yourself, in order.
- The task is small enough to do directly (a single lookup, a quick read, a one-line edit). Do not use a subagent to avoid doing the task yourself.
- You only want to parallelize reads of the same files; batch those search/read calls in one message instead.

## How to dispatch: all of them in one call

`tasks` is an array, and **one call can carry several subagents**. Put **all** the independent subagents into the `tasks` array of a **single** `subagent` call — do not dispatch one and wait before the next, and do not split them across several calls.

Each dispatch ends your turn, so dispatching one at a time means you cannot start the rest until the previous subagent has finished and the conversation has resumed.
