import { createContext } from 'svelte';

// One dialog in the application shell serves both the sidebar and overview.
export const [useCreateWorkspace, provideCreateWorkspace] = createContext<() => void>();
