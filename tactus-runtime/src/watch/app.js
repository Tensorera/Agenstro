'use strict';
const $ = (id) => document.getElementById(id);
const token = new URLSearchParams(location.hash.slice(1)).get('token') || '';
let selected = null;
let liveAfter = '0';
let historyAfter = '0';
let generation = 0;
let lastChunk = null;
let renderedBytes = 0;
let historyComplete = false;
let polling = false;
let tick = 0;

async function get(path) {
  const response = await fetch(path, {headers: {Authorization: `Bearer ${token}`}, cache: 'no-store', signal: AbortSignal.timeout(5000)});
  if (!response.ok) throw new Error(response.status === 401 ? 'Open the complete URL printed by tactus watch.' : `Viewer request failed (${response.status}).`);
  return response.json();
}

function clearOutput() {
  $('output').replaceChildren();
  lastChunk = null;
  renderedBytes = 0;
}

function select(run) {
  selected = run;
  generation += 1;
  liveAfter = '0';
  historyAfter = '0';
  historyComplete = false;
  clearOutput();
  $('events').replaceChildren();
  $('notice').textContent = '';
  $('subject').textContent = run || 'All live output';
  $('all').classList.toggle('selected', run === null);
  for (const button of $('runs').children) button.classList.toggle('selected', button.dataset.run === run);
  $('history-state').textContent = run ? 'Reading journal…' : 'Select a run to inspect its journal.';
}

function append(event) {
  $('output').querySelector('.empty')?.remove();
  const key = `${event.runId}/${event.kind}`;
  if (!lastChunk || lastChunk.key !== key || event.kind !== 'text.delta') {
    const chunk = document.createElement('article');
    chunk.className = 'chunk';
    const meta = document.createElement('div');
    meta.className = 'metadata';
    meta.textContent = `${new Date(Number(event.atUnixMs)).toLocaleTimeString()} · ${event.provider} · ${event.runId} · ${event.kind}`;
    const pre = document.createElement('pre');
    pre.className = 'text';
    chunk.append(meta, pre);
    $('output').append(chunk);
    lastChunk = {key, pre};
  }
  lastChunk.pre.append(document.createTextNode(event.text));
  renderedBytes += event.text.length;
  if (event.truncated || event.missedPackets) $('notice').textContent = 'Some live output was truncated or dropped. Check the run journal for its recorded status.';
  while (renderedBytes > 2000000 || $('output').childElementCount > 1000) {
    const oldest = $('output').firstElementChild;
    if (!oldest || oldest.contains(lastChunk.pre)) {
      clearOutput();
      break;
    }
    renderedBytes -= oldest.querySelector('pre')?.textContent.length || 0;
    oldest.remove();
    $('notice').textContent = 'Older output was removed to keep the display bounded.';
  }
}

async function updateRuns() {
  const data = await get('/api/runs');
  $('workspace').textContent = data.workspace.name;
  $('count').textContent = `(${data.runs.length})`;
  const existing = new Map(Array.from($('runs').children, (button) => [button.dataset.run, button]));
  const buttons = data.runs.map((run) => {
    let button = existing.get(run.runId);
    if (!button) {
      button = document.createElement('button');
      button.dataset.run = run.runId;
      button.append(document.createTextNode(''), document.createElement('small'));
      button.addEventListener('click', () => select(run.runId));
    }
    button.classList.toggle('selected', selected === run.runId);
    button.firstChild.nodeValue = run.label;
    button.lastChild.textContent = `${run.state} · ${run.runId}`;
    existing.delete(run.runId);
    return button;
  });
  for (const button of existing.values()) button.remove();
  // Preserve nodes and focus across status refreshes. Move only reordered runs.
  buttons.forEach((button, index) => {
    if ($('runs').children[index] !== button) $('runs').insertBefore(button, $('runs').children[index] || null);
  });
}

async function updateLive() {
  const current = generation;
  const data = await get(`/api/live?after=${liveAfter}${selected ? `&run=${encodeURIComponent(selected)}` : ''}`);
  if (current !== generation) return;
  liveAfter = data.nextAfter;
  if (data.historyLost) $('notice').textContent = 'Older output has expired from the viewer’s memory. Showing retained output.';
  for (const event of data.events) append(event);
  if ($('follow').checked && data.events.length) $('output').scrollTop = $('output').scrollHeight;
}

async function updateHistory() {
  if (!selected || historyComplete) return;
  const current = generation;
  const data = await get(`/api/events?run=${encodeURIComponent(selected)}&after=${historyAfter}`);
  if (current !== generation) return;
  historyAfter = data.nextAfter;
  historyComplete = data.complete;
  $('history-state').textContent = `${data.run.state} · ${data.integrity}${data.complete ? ' · journal complete' : ' · awaiting more journal events'}`;
  for (const event of data.events) {
    const row = document.createElement('li');
    row.textContent = `#${event.seq} ${event.kind}${event.presentation?.message ? ` — ${event.presentation.message}` : ''}`;
    $('events').append(row);
  }
  while ($('events').childElementCount > 500) $('events').firstElementChild.remove();
}

async function poll() {
  if (polling) return;
  polling = true;
  try {
    await updateLive();
    if (tick++ % 4 === 0) await Promise.all([updateRuns(), updateHistory()]);
    $('connection').textContent = 'Connected · local';
  } catch (error) {
    $('connection').textContent = 'Reconnecting…';
    $('notice').textContent = error.message;
  } finally {
    polling = false;
  }
}

$('all').addEventListener('click', () => select(null));
$('clear').addEventListener('click', clearOutput);
if (!token) {
  $('connection').textContent = 'URL token required';
  $('notice').textContent = 'Open the complete URL printed by tactus watch in your terminal.';
} else {
  poll();
  setInterval(poll, 500);
}
