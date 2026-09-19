import { create } from 'zustand';
import type { TodoItem } from '@flamel/shared';

interface TodoStore {
  todosByProject: Record<string, TodoItem[]>;
  setTodos: (projectPath: string, todos: TodoItem[]) => void;
  addTodo: (projectPath: string, todo: TodoItem) => void;
  updateTodo: (projectPath: string, id: string, patch: Partial<TodoItem>) => void;
  removeTodo: (projectPath: string, id: string) => void;
  clearDone: (projectPath: string) => void;
}

export const useTodoStore = create<TodoStore>((set) => ({
  todosByProject: {},

  setTodos: (projectPath, todos) =>
    set(s => ({ todosByProject: { ...s.todosByProject, [projectPath]: todos } })),

  addTodo: (projectPath, todo) =>
    set(s => ({
      todosByProject: {
        ...s.todosByProject,
        [projectPath]: [...(s.todosByProject[projectPath] || []), todo],
      },
    })),

  updateTodo: (projectPath, id, patch) =>
    set(s => ({
      todosByProject: {
        ...s.todosByProject,
        [projectPath]: (s.todosByProject[projectPath] || []).map(t =>
          t.id === id ? { ...t, ...patch } : t
        ),
      },
    })),

  removeTodo: (projectPath, id) =>
    set(s => ({
      todosByProject: {
        ...s.todosByProject,
        [projectPath]: (s.todosByProject[projectPath] || []).filter(t => t.id !== id),
      },
    })),

  clearDone: (projectPath) =>
    set(s => ({
      todosByProject: {
        ...s.todosByProject,
        [projectPath]: (s.todosByProject[projectPath] || []).filter(t => !t.done),
      },
    })),
}));
