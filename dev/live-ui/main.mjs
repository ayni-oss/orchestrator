import './style.css';

const updates = (import.meta.hot?.data.updates || 0) + 1;
document.querySelector('#hmr').textContent = `Frontend loaded · module revision ${updates}`;
const controller = new AbortController();

async function refreshStatus() {
  try {
    const response = await fetch('/__live/status', { signal: controller.signal });
    if (!response.ok) throw new Error('Live status unavailable');
    const result = await response.json();
    document.querySelector('#freshness').textContent = result.stale || !result.status?.owner_pid
      ? 'Status unavailable or stale. The backend may still be serving an earlier build.'
      : `Live loop observed at ${result.observedAt}`;
    document.querySelector('#build').textContent = JSON.stringify(result.status, null, 2);
  } catch (error) {
    if (error.name !== 'AbortError') document.querySelector('#freshness').textContent = 'Status connection lost; retrying.';
  }
}

async function refreshWorkspaces() {
  const button = document.querySelector('#refresh');
  button.disabled = true;
  document.querySelector('#api-status').textContent = 'Reading canonical workspace list…';
  try {
    const response = await fetch('/api/ListWorkspaces', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json', 'X-Console-Request': '1' },
      body: JSON.stringify({ limit: 20 }),
      signal: controller.signal,
    });
    const result = await response.json();
    if (!response.ok) throw new Error(`API returned ${response.status}`);
    document.querySelector('#api-status').textContent = 'Canonical API responded. This read does not authorize work.';
    document.querySelector('#workspaces').textContent = JSON.stringify(result, null, 2);
  } catch (error) {
    if (error.name !== 'AbortError') {
      document.querySelector('#api-status').textContent = `${error.message}. Retry after the gateway/backend is ready.`;
      document.querySelector('#workspaces').textContent = '';
    }
  } finally {
    button.disabled = false;
  }
}

const button = document.querySelector('#refresh');
button.addEventListener('click', refreshWorkspaces);
void refreshStatus();
const timer = setInterval(refreshStatus, 3000);
if (import.meta.hot) {
  import.meta.hot.accept();
  import.meta.hot.dispose((data) => {
    data.updates = updates;
    clearInterval(timer);
    controller.abort();
    button.removeEventListener('click', refreshWorkspaces);
  });
}
