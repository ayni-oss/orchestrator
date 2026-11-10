import { beforeNavigate } from '$app/navigation';
import { onMount } from 'svelte';

/** Register once per editor; the predicate is evaluated at navigation time. */
export function protectUnsavedChanges(hasUnsavedChanges: () => boolean) {
  beforeNavigate(({ cancel, willUnload, shallow }) => {
    if (shallow || !hasUnsavedChanges()) return;
    if (willUnload || !window.confirm('Changes have not been saved. Leave this page?')) cancel();
  });
  onMount(() => {
    const warn = (event: BeforeUnloadEvent) => {
      if (!hasUnsavedChanges()) return;
      event.preventDefault();
      event.returnValue = '';
    };
    window.addEventListener('beforeunload', warn);
    return () => window.removeEventListener('beforeunload', warn);
  });
}
