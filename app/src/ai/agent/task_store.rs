use std::collections::{HashMap, HashSet};

use indexmap::IndexMap;
use warp_multi_agent_api as api;

use super::{
    task::{
        helper::{MessageExt, ToolCallExt},
        Task, TaskId,
    },
    AIAgentExchange, AIAgentExchangeId, AIAgentOutputMessageType,
};

#[derive(Debug, Clone)]
struct ExchangeRef {
    task_id: TaskId,
    exchange_index: usize,
}

/// Task storage with a linearized exchange index for O(1) first/last access.
#[derive(Debug, Clone)]
pub struct TaskStore {
    root_task_id: TaskId,
    tasks: HashMap<TaskId, Task>,
    exchanges: IndexMap<AIAgentExchangeId, ExchangeRef>,
}

impl TaskStore {
    pub fn with_root_task(root_task: Task) -> Self {
        let root_task_id = root_task.id().clone();
        let mut store = Self {
            tasks: HashMap::new(),
            exchanges: Default::default(),
            root_task_id: root_task_id.clone(),
        };
        store.tasks.insert(root_task_id, root_task);
        store.rebuild_exchange_index();
        store
    }

    /// Creates a TaskStore from an existing HashMap of tasks.
    /// Rebuilds the linearized index after construction.
    pub fn from_tasks(tasks: HashMap<TaskId, Task>, root_task_id: TaskId) -> Self {
        let mut store = Self {
            tasks,
            exchanges: Default::default(),
            root_task_id,
        };
        store.rebuild_exchange_index();
        store
    }

    pub fn root_task_id(&self) -> &TaskId {
        &self.root_task_id
    }

    pub fn get(&self, task_id: &TaskId) -> Option<&Task> {
        self.tasks.get(task_id)
    }

    pub fn tasks(&self) -> impl Iterator<Item = &Task> {
        self.tasks.values()
    }

    pub fn task_count(&self) -> usize {
        self.tasks.len()
    }

    /// Appends an exchange to a task and updates the index.
    /// Returns true if the task was found and the exchange was appended.
    pub fn append_exchange(&mut self, task_id: &TaskId, exchange: AIAgentExchange) -> bool {
        let Some(task) = self.tasks.get_mut(task_id) else {
            return false;
        };
        let exchange_index = task.exchanges_len();
        let exchange_id = exchange.id;
        let has_subagent_output = exchange.output_status.output().is_some_and(|output| {
            output
                .get()
                .messages
                .iter()
                .any(|m| matches!(m.message, AIAgentOutputMessageType::Subagent(_)))
        });
        task.append_exchange(exchange);
        let is_at_dfs_tail = self
            .exchanges
            .last()
            .is_none_or(|(_, r)| r.task_id == *task_id);
        // If we know this exchange is going at the end, it's faster to just append it.
        if is_at_dfs_tail && !has_subagent_output {
            self.exchanges.insert(
                exchange_id,
                ExchangeRef {
                    task_id: task_id.clone(),
                    exchange_index,
                },
            );
        } else {
            // Otherwise, we need to rebuild the whole index.
            self.rebuild_exchange_index();
        }
        true
    }

    /// Removes an exchange from a task and rebuilds the index.
    /// Returns the removed exchange if found.
    pub fn remove_task_exchange(
        &mut self,
        task_id: &TaskId,
        exchange_id: AIAgentExchangeId,
    ) -> Option<AIAgentExchange> {
        let task = self.tasks.get_mut(task_id)?;
        let exchange = task.remove_exchange(exchange_id)?;
        self.rebuild_exchange_index();
        Some(exchange)
    }

    /// Returns a mutable reference to an exchange by its ID, searching all tasks.
    pub fn exchange_mut(&mut self, exchange_id: AIAgentExchangeId) -> Option<&mut AIAgentExchange> {
        for task in self.tasks.values_mut() {
            if let Some(exchange) = task.exchange_mut(exchange_id) {
                return Some(exchange);
            }
        }
        None
    }

    /// Modifies a task via the provided closure and rebuilds the exchange index
    /// if exchanges changed.
    pub fn modify_task<R>(
        &mut self,
        task_id: &TaskId,
        f: impl FnOnce(&mut Task) -> R,
    ) -> Option<R> {
        let exchange_count_before = self.tasks.get(task_id)?.exchanges_len();
        let task = self.tasks.get_mut(task_id)?;
        let result = f(task);
        let exchange_count_after = self
            .tasks
            .get(task_id)
            .map(|t| t.exchanges_len())
            .unwrap_or(0);
        if exchange_count_before != exchange_count_after {
            self.rebuild_exchange_index();
        }
        Some(result)
    }

    /// Modifies the root task via the provided closure and rebuilds the exchange index if exchanges changed.
    pub fn modify_root_task<R>(&mut self, f: impl FnOnce(&mut Task) -> R) -> Option<R> {
        let root_task_id = self.root_task_id.clone();
        self.modify_task(&root_task_id, f)
    }

    pub fn root_task(&self) -> Option<&Task> {
        self.tasks.get(&self.root_task_id)
    }

    /// Sets or replaces the root task, removing any previous root if it exists.
    pub fn set_root_task(&mut self, root_task: Task) {
        // Remove the old root task and its exchange refs
        let old_root_id = self.root_task_id.clone();
        self.remove(&old_root_id);

        let new_root_id = root_task.id().clone();
        self.root_task_id = new_root_id;
        self.insert(root_task);
    }

    pub fn exchange_by_id(&self, exchange_id: AIAgentExchangeId) -> Option<&AIAgentExchange> {
        // Fast path: exchanges reachable from the root DFS traversal.
        if let Some(exchange_ref) = self.exchanges.get(&exchange_id) {
            return self.lookup_exchange(exchange_ref);
        }
        // Fallback: an optimistically-created subtask can exist before its
        // parent's sub-agent call has streamed in, so its exchanges are absent
        // from the linearized index but must still be resolvable by id
        // (e.g. `AIBlockModelImpl::new`).
        self.tasks
            .values()
            .find_map(|task| task.exchange(exchange_id))
    }

    pub fn first_exchange(&self) -> Option<&AIAgentExchange> {
        self.exchanges
            .first()
            .and_then(|(_, v)| self.lookup_exchange(v))
    }

    pub fn latest_exchange(&self) -> Option<&AIAgentExchange> {
        self.exchanges
            .last()
            .and_then(|(_, v)| self.lookup_exchange(v))
    }

    pub fn exchange_count(&self) -> usize {
        self.exchanges.len()
    }

    pub fn all_exchanges(&self) -> impl Iterator<Item = &AIAgentExchange> {
        self.exchanges
            .values()
            .filter_map(|r| self.lookup_exchange(r))
    }

    pub fn all_exchanges_rev(&self) -> impl Iterator<Item = &AIAgentExchange> {
        self.exchanges
            .values()
            .rev()
            .filter_map(|r| self.lookup_exchange(r))
    }

    pub fn all_exchanges_by_task(&self) -> Vec<(TaskId, Vec<&AIAgentExchange>)> {
        let mut result: Vec<(TaskId, Vec<&AIAgentExchange>)> = Vec::new();

        for exchange_ref in self.exchanges.values() {
            let Some(exchange) = self.lookup_exchange(exchange_ref) else {
                continue;
            };

            // Check if we should append to the last group or start a new one
            if let Some((last_task_id, exchanges)) = result.last_mut() {
                if last_task_id == &exchange_ref.task_id {
                    exchanges.push(exchange);
                    continue;
                }
            }

            // Start a new group
            result.push((exchange_ref.task_id.clone(), vec![exchange]));
        }

        result
    }

    /// Returns all messages in linearized DFS order, interleaving subtask messages
    /// immediately after their parent subagent call messages.
    pub fn all_linearized_messages(&self) -> Vec<&api::Message> {
        fn collect_messages_dfs<'a>(
            me: &'a TaskStore,
            messages: &mut Vec<&'a api::Message>,
            task: &'a Task,
        ) {
            for message in task.messages() {
                messages.push(message);
                // If this message is a subagent call, recursively add subtask messages
                if let Some(subagent_call) = message
                    .tool_call()
                    .and_then(|tc: &api::message::ToolCall| tc.subagent())
                {
                    if let Some(subtask) = me.get(&TaskId::new(subagent_call.task_id.clone())) {
                        collect_messages_dfs(me, messages, subtask);
                    }
                }
            }
        }

        let mut messages = Vec::new();
        if let Some(root_task) = self.root_task() {
            collect_messages_dfs(self, &mut messages, root_task);
        }
        messages
    }

    pub fn insert(&mut self, task: Task) {
        self.tasks.insert(task.id().clone(), task);
        self.rebuild_exchange_index();
    }

    pub fn remove(&mut self, task_id: &TaskId) -> Option<Task> {
        let task = self.tasks.remove(task_id)?;
        self.exchanges.retain(|_, r| &r.task_id != task_id);
        Some(task)
    }

    fn lookup_exchange(&self, r: &ExchangeRef) -> Option<&AIAgentExchange> {
        self.tasks
            .get(&r.task_id)?
            .exchanges()
            .nth(r.exchange_index)
    }

    /// Rebuilds the linearized index from scratch using DFS traversal.
    pub(super) fn rebuild_exchange_index(&mut self) {
        self.exchanges = Self::build_exchange_index(&self.tasks, &self.root_task_id);
    }

    /// Builds linearized exchange refs via DFS traversal without mutating self.
    /// This allows us to borrow `tasks` immutably throughout the traversal.
    fn build_exchange_index(
        tasks: &HashMap<TaskId, Task>,
        root_task_id: &TaskId,
    ) -> IndexMap<AIAgentExchangeId, ExchangeRef> {
        let mut refs = IndexMap::new();

        fn append_refs_for_task(
            tasks: &HashMap<TaskId, Task>,
            refs: &mut IndexMap<AIAgentExchangeId, ExchangeRef>,
            task: &Task,
        ) {
            let task_id = task.id().clone();

            for (exchange_index, exchange) in task.exchanges().enumerate() {
                refs.insert(
                    exchange.id,
                    ExchangeRef {
                        task_id: task_id.clone(),
                        exchange_index,
                    },
                );

                // Check for subagent calls in the exchange output.
                if let Some(output) = exchange.output_status.output() {
                    for output_message in output.get().messages.iter() {
                        if let AIAgentOutputMessageType::Subagent(subagent_call) =
                            &output_message.message
                        {
                            if let Some(subtask) =
                                tasks.get(&TaskId::new(subagent_call.task_id.clone()))
                            {
                                append_refs_for_task(tasks, refs, subtask);
                            }
                        }
                    }
                }
            }
        }

        if let Some(root_task) = tasks.get(root_task_id) {
            append_refs_for_task(tasks, &mut refs, root_task);
        }

        refs
    }
    pub fn prune_unreachable_subtasks(&mut self) {
        let reachable = self.reachable_task_ids();
        let root_task_id = &self.root_task_id.clone();
        let to_remove: Vec<TaskId> = self
            .tasks
            .keys()
            .filter(|id| **id != *root_task_id && !reachable.contains(*id))
            .cloned()
            .collect();
        if to_remove.is_empty() {
            return;
        }
        for id in &to_remove {
            self.tasks.remove(id);
        }
        self.exchanges
            .retain(|_, r| self.tasks.contains_key(&r.task_id));
    }

    fn reachable_task_ids(&self) -> HashSet<TaskId> {
        let mut reachable: HashSet<TaskId> = HashSet::new();
        let mut queue = vec![self.root_task_id.clone()];
        while let Some(task_id) = queue.pop() {
            if !reachable.insert(task_id.clone()) {
                continue;
            }
            let Some(task) = self.tasks.get(&task_id) else {
                continue;
            };
            for message in task.messages() {
                let tc = message.tool_call();
                if let Some(tc) = tc {
                    if let Some(subagent) = tc.subagent() {
                        if !subagent.task_id.is_empty() {
                            queue.push(TaskId::new(subagent.task_id.clone()));
                        }
                    }
                }
            }
        }
        reachable
    }
}

#[cfg(test)]
mod testing {
    use crate::ai::agent::task::TaskId;

    use super::TaskStore;

    impl TaskStore {
        pub fn contains(&self, task_id: &TaskId) -> bool {
            self.tasks.contains_key(task_id)
        }
    }
}

#[cfg(test)]
#[path = "task_store_tests.rs"]
mod tests;
