'use strict';

// ---------------------------------------------------------------------------
// Canvas and state
// ---------------------------------------------------------------------------

const canvas = document.getElementById('map');
const ctx = canvas.getContext('2d');
const baseCanvas = document.createElement('canvas');
const baseCtx = baseCanvas.getContext('2d');

const RES_KEYS = ['food', 'wood', 'stone', 'copper', 'iron', 'gold'];
const RES_LABELS = {
  food: 'Food',
  wood: 'Wood',
  stone: 'Stone',
  copper: 'Copper',
  iron: 'Iron',
  gold: 'Gold',
};
const RES_SHORT = { food: 'F', wood: 'W', stone: 'S', copper: 'C', iron: 'I', gold: 'G' };
const PALETTE = [
  '#ff6b6b',
  '#4aa3ff',
  '#51cf66',
  '#fcc419',
  '#cc5de8',
  '#ff922b',
  '#20c997',
  '#dee2e6',
];
const TERRAIN_COLORS = { g: '#3e6b3d', w: '#1c3d5c', r: '#4b4b4b' };
const NODE_COLORS = {
  food: '#d9b44a',
  wood: '#2f8f5b',
  stone: '#9aa0a6',
  copper: '#c97b3a',
  iron: '#7f8c9a',
  gold: '#e8c547',
};

let state = null;
let view = { tile: 1, ox: 0, oy: 0 };
let selected = new Set();
let buildDef = null;
let hover = null;
let drag = null;
let placing = false;
let baseDirty = true;
let nodeTiles = new Set();
let polling = false;
let lastLog = '';
let toastTimer = 0;
let offline = false;

// ---------------------------------------------------------------------------
// HTTP helpers
// ---------------------------------------------------------------------------

async function pollState() {
  if (polling) return;
  polling = true;
  try {
    const response = await fetch('/api/state', { cache: 'no-store' });
    if (!response.ok) throw new Error(response.status);
    applyState(await response.json());
    offline = false;
  } catch (error) {
    if (!offline) {
      offline = true;
      document.getElementById('outcome').textContent = 'server offline';
    }
  } finally {
    polling = false;
  }
}

function applyState(next) {
  state = next;
  pruneSelection();
  if (buildDef && !state.catalog.buildings.some((building) => building.id === buildDef.id)) {
    buildDef = null;
  }
  nodeTiles = new Set(state.map.nodes.map((node) => `${node.x},${node.y}`));
  baseDirty = true;
  updateAll();
}

async function sendCommand(command) {
  try {
    const response = await fetch('/api/command', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ command }),
    });
    const result = await response.json();
    if (!result.accepted) {
      toast(`Rejected: ${prettyReason(result.reason)}`);
    }
    pollState();
    return result;
  } catch (error) {
    toast('Command failed');
    return null;
  }
}

async function restartMatch() {
  try {
    const response = await fetch('/api/restart', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ seed: Math.floor(Math.random() * 1_000_000_000) }),
    });
    if (!response.ok) throw new Error(response.status);
    selected.clear();
    buildDef = null;
    drag = null;
    applyState(await response.json());
    toast('New match started');
  } catch (error) {
    toast('Restart failed');
  }
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

function formatTime(seconds) {
  const minutes = Math.floor(seconds / 60);
  const rest = Math.floor(seconds % 60);
  return `${minutes}:${String(rest).padStart(2, '0')}`;
}

function outcomeText(outcome) {
  if (outcome === 'Ongoing') return 'in progress';
  if (outcome === 'Finished') return 'finished';
  if (outcome && typeof outcome === 'object' && 'Victory' in outcome) {
    return `victory: P${outcome.Victory}`;
  }
  return String(outcome);
}

function prettyReason(reason) {
  if (reason == null) return 'unknown';
  return String(reason).replace(/([a-z])([A-Z])/g, '$1 $2');
}

function costText(cost) {
  const parts = [];
  for (const key of RES_KEYS) {
    const value = cost[key] || 0;
    if (value > 0) parts.push(`${value}${RES_SHORT[key]}`);
  }
  return parts.length ? parts.join(' ') : 'free';
}

function canAfford(cost) {
  if (!state) return false;
  for (const key of RES_KEYS) {
    if ((cost[key] || 0) > (state.player.resources[key] || 0)) return false;
  }
  return true;
}

function keyOf(id) {
  if (id == null) return '';
  if (typeof id === 'object') return `${id.index}.${id.generation}`;
  return String(id);
}

function ownerColor(owner) {
  if (!state) return '#fff';
  const player = state.players.find((candidate) => candidate.id === owner);
  const index = player ? player.color : owner;
  return PALETTE[Math.abs(index) % PALETTE.length];
}

function withAlpha(hex, alpha) {
  const value = parseInt(hex.slice(1), 16);
  const r = (value >> 16) & 255;
  const g = (value >> 8) & 255;
  const b = value & 255;
  return `rgba(${r},${g},${b},${alpha})`;
}

function toast(message) {
  const element = document.getElementById('toast');
  element.textContent = message;
  element.classList.add('show');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => element.classList.remove('show'), 2200);
}

// ---------------------------------------------------------------------------
// Selection helpers
// ---------------------------------------------------------------------------

function pruneSelection() {
  if (!state) return;
  const present = new Set(state.entities.map((entity) => keyOf(entity.id)));
  for (const key of [...selected]) {
    if (!present.has(key)) selected.delete(key);
  }
}

function selectedEntities() {
  if (!state) return [];
  return state.entities.filter((entity) => selected.has(keyOf(entity.id)));
}

function selectedUnits() {
  return selectedEntities().filter(
    (entity) => entity.kind === 'unit' && entity.owner === state.human,
  );
}

function selectedBuildings() {
  return selectedEntities().filter(
    (entity) => entity.kind === 'building' && entity.owner === state.human,
  );
}

function entityAt(tile) {
  if (!state || !tile) return null;
  for (const entity of state.entities) {
    if (entity.kind !== 'building') continue;
    if (
      tile.x >= entity.origin_x &&
      tile.x < entity.origin_x + entity.footprint &&
      tile.y >= entity.origin_y &&
      tile.y < entity.origin_y + entity.footprint
    ) {
      return entity;
    }
  }
  let best = null;
  let bestDistance = 0.9;
  for (const entity of state.entities) {
    if (entity.kind !== 'unit') continue;
    const dx = entity.x - (tile.x + 0.5);
    const dy = entity.y - (tile.y + 0.5);
    const distance = Math.hypot(dx, dy);
    if (distance < bestDistance) {
      bestDistance = distance;
      best = entity;
    }
  }
  return best;
}

function nodeAt(tile) {
  if (!state || !tile) return null;
  return state.map.nodes.find((node) => node.x === tile.x && node.y === tile.y) || null;
}

function clickSelect(tile, shift) {
  if (!state) return;
  if (!tile) {
    if (!shift) selected.clear();
    updateSelection();
    draw();
    return;
  }
  const entity = entityAt(tile);
  if (entity && entity.owner === state.human) {
    const key = keyOf(entity.id);
    if (shift) {
      if (selected.has(key)) selected.delete(key);
      else selected.add(key);
    } else {
      selected = new Set([key]);
    }
  } else if (!shift) {
    selected.clear();
  }
  updateSelection();
  updateHint();
  draw();
}

function boxSelect(box) {
  if (!state) return;
  const a = tileAt({ x: box.x0, y: box.y0 });
  const b = tileAt({ x: box.x1, y: box.y1 });
  if (!a || !b) return;
  const x0 = Math.min(a.x, b.x);
  const x1 = Math.max(a.x, b.x);
  const y0 = Math.min(a.y, b.y);
  const y1 = Math.max(a.y, b.y);
  const picked = state.entities
    .filter((entity) => {
      if (entity.kind !== 'unit' || entity.owner !== state.human) return false;
      const x = Math.floor(entity.x);
      const y = Math.floor(entity.y);
      return x >= x0 && x <= x1 && y >= y0 && y <= y1;
    })
    .map((entity) => keyOf(entity.id));
  if (!box.shift) selected.clear();
  for (const key of picked) selected.add(key);
  updateSelection();
  updateHint();
  draw();
}

// ---------------------------------------------------------------------------
// Context orders and building placement
// ---------------------------------------------------------------------------

function onContextMenu(event) {
  event.preventDefault();
  if (!state) return;
  if (buildDef) {
    buildDef = null;
    updateBuild();
    updateHint();
    draw();
    return;
  }
  const tile = tileAt(point(event));
  if (!tile) return;

  const units = selectedUnits();
  const entities = selectedEntities();

  if (units.length) {
    const node = nodeAt(tile);
    const entity = entityAt(tile);
    const workers = units.filter((unit) => unit.role === 'Worker');
    if (node && workers.length) {
      sendCommand({
        Gather: { units: workers.map((unit) => unit.id), node: node.i },
      });
      return;
    }
    if (entity && entity.owner !== state.human) {
      sendCommand({ Attack: { units: units.map((unit) => unit.id), target: entity.id } });
      return;
    }
    const order = event.ctrlKey || event.shiftKey ? 'AttackMove' : 'Move';
    sendCommand({
      [order]: {
        units: units.map((unit) => unit.id),
        target: { x: tile.x, y: tile.y },
      },
    });
    return;
  }

  const buildings = entities.filter((entity) => entity.kind === 'building');
  if (buildings.length) {
    sendCommand({
      SetRally: {
        building: buildings[0].id,
        target: { x: tile.x, y: tile.y },
      },
    });
  }
}

function buildingOrigin(tile, footprint) {
  const maxX = state.map.width - 1 - footprint;
  const maxY = state.map.height - 1 - footprint;
  return {
    x: Math.max(1, Math.min(maxX, tile.x - Math.floor((footprint - 1) / 2))),
    y: Math.max(1, Math.min(maxY, tile.y - Math.floor((footprint - 1) / 2))),
  };
}

function canPlace(origin, footprint) {
  if (!state) return false;
  if (origin.x < 1 || origin.y < 1) return false;
  if (origin.x + footprint > state.map.width - 1) return false;
  if (origin.y + footprint > state.map.height - 1) return false;
  for (let y = origin.y; y < origin.y + footprint; y++) {
    for (let x = origin.x; x < origin.x + footprint; x++) {
      if (state.map.terrain[y][x] !== 'g') return false;
      if (nodeTiles.has(`${x},${y}`)) return false;
      for (const entity of state.entities) {
        if (entity.kind !== 'building') continue;
        if (
          x >= entity.origin_x &&
          x < entity.origin_x + entity.footprint &&
          y >= entity.origin_y &&
          y < entity.origin_y + entity.footprint
        ) {
          return false;
        }
      }
    }
  }
  return true;
}

async function placeBuilding(tile) {
  if (!buildDef || !state) return;
  const definition = buildDef;
  const origin = buildingOrigin(tile, definition.footprint);
  const result = await sendCommand({
    PlaceBuilding: { building: definition.id, origin },
  });
  if (result && result.accepted) {
    buildDef = null;
  } else {
    buildDef = definition;
  }
  updateBuild();
  updateHint();
  draw();
}

// ---------------------------------------------------------------------------
// Mouse input
// ---------------------------------------------------------------------------

function point(event) {
  const rect = canvas.getBoundingClientRect();
  return { x: event.clientX - rect.left, y: event.clientY - rect.top };
}

function tileAt(pt) {
  if (!state) return null;
  const x = Math.floor((pt.x - view.ox) / view.tile);
  const y = Math.floor((pt.y - view.oy) / view.tile);
  if (x < 0 || y < 0 || x >= state.map.width || y >= state.map.height) return null;
  return { x, y };
}

function onMouseDown(event) {
  if (!state || event.button !== 0) return;
  const tile = tileAt(point(event));
  if (!tile) return;
  hover = tile;
  if (buildDef) {
    placing = true;
    draw();
    return;
  }
  drag = {
    x0: point(event).x,
    y0: point(event).y,
    x1: point(event).x,
    y1: point(event).y,
    shift: event.shiftKey,
    moved: false,
  };
  draw();
}

function onMouseMove(event) {
  if (!state) return;
  const pt = point(event);
  hover = tileAt(pt);
  if (drag) {
    drag.x1 = pt.x;
    drag.y1 = pt.y;
    drag.moved = Math.hypot(drag.x1 - drag.x0, drag.y1 - drag.y0) > 5;
  }
  updateHint();
  draw();
}

function onMouseUp(event) {
  if (!state || event.button !== 0) return;
  if (buildDef) {
    if (placing) {
      placing = false;
      const tile = tileAt(point(event));
      if (tile) placeBuilding(tile);
      else toast('Click inside the map');
    }
    return;
  }
  if (!drag) return;
  const box = drag;
  drag = null;
  if (box.moved) {
    boxSelect(box);
  } else {
    const tile = tileAt(point(event));
    clickSelect(tile, box.shift);
  }
  draw();
}

// ---------------------------------------------------------------------------
// Rendering
// ---------------------------------------------------------------------------

function resizeCanvas() {
  const width = canvas.clientWidth;
  const height = canvas.clientHeight;
  if (!width || !height) return;
  const dpr = window.devicePixelRatio || 1;
  canvas.width = Math.max(1, Math.floor(width * dpr));
  canvas.height = Math.max(1, Math.floor(height * dpr));
  baseCanvas.width = canvas.width;
  baseCanvas.height = canvas.height;
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  baseDirty = true;
  draw();
}

function rebuildBase() {
  if (!state) return;
  const width = canvas.clientWidth;
  const height = canvas.clientHeight;
  if (!width || !height) return;
  const dpr = window.devicePixelRatio || 1;
  baseCtx.setTransform(dpr, 0, 0, dpr, 0, 0);
  baseCtx.clearRect(0, 0, width, height);
  baseCtx.fillStyle = '#05080b';
  baseCtx.fillRect(0, 0, width, height);

  const map = state.map;
  const tile = Math.max(1, Math.min(width / map.width, height / map.height));
  const ox = (width - map.width * tile) / 2;
  const oy = (height - map.height * tile) / 2;
  view = { tile, ox, oy };

  for (let y = 0; y < map.height; y++) {
    const terrainRow = map.terrain[y];
    const visibleRow = map.visible[y];
    const exploredRow = map.explored[y];
    for (let x = 0; x < map.width; x++) {
      const px = Math.floor(ox + x * tile);
      const py = Math.floor(oy + y * tile);
      const size = Math.ceil(tile) + 1;
      if (exploredRow[x] !== '1') {
        baseCtx.fillStyle = '#000';
        baseCtx.fillRect(px, py, size, size);
        continue;
      }
      baseCtx.fillStyle = TERRAIN_COLORS[terrainRow[x]] || '#333';
      baseCtx.fillRect(px, py, size, size);
      if (visibleRow[x] !== '1') {
        baseCtx.fillStyle = 'rgba(0,0,0,0.48)';
        baseCtx.fillRect(px, py, size, size);
      }
    }
  }

  for (const node of map.nodes) {
    if (map.explored[node.y][node.x] !== '1') continue;
    const visible = map.visible[node.y][node.x] === '1';
    const radius = Math.max(1.4, tile * 0.23);
    baseCtx.globalAlpha = visible ? 1 : 0.55;
    baseCtx.fillStyle = NODE_COLORS[node.kind] || '#fff';
    baseCtx.beginPath();
    baseCtx.arc(
      ox + (node.x + 0.5) * tile,
      oy + (node.y + 0.5) * tile,
      radius,
      0,
      Math.PI * 2,
    );
    baseCtx.fill();
    baseCtx.globalAlpha = 1;
  }

  baseDirty = false;
}

function draw() {
  if (!state) return;
  const width = canvas.clientWidth;
  const height = canvas.clientHeight;
  if (!width || !height) return;
  if (baseDirty) rebuildBase();
  ctx.clearRect(0, 0, width, height);
  ctx.drawImage(baseCanvas, 0, 0, width, height);

  drawEntities();
  drawSelectionBox();
  drawBuildGhost();
  drawHover();
}

function drawEntities() {
  if (!state) return;
  const tile = view.tile;

  for (const entity of state.entities) {
    if (entity.kind !== 'building') continue;
    const x = view.ox + entity.origin_x * tile;
    const y = view.oy + entity.origin_y * tile;
    const size = entity.footprint * tile;
    const color = ownerColor(entity.owner);
    ctx.fillStyle = withAlpha(color, entity.finished ? 0.52 : 0.25);
    ctx.fillRect(x + 1, y + 1, Math.max(1, size - 2), Math.max(1, size - 2));
    ctx.lineWidth = 1.5;
    ctx.strokeStyle = color;
    ctx.strokeRect(x + 1.5, y + 1.5, Math.max(1, size - 3), Math.max(1, size - 3));
    if (!entity.finished) {
      ctx.setLineDash([4, 3]);
      ctx.strokeStyle = '#fcc419';
      ctx.strokeRect(x + 2.5, y + 2.5, Math.max(1, size - 5), Math.max(1, size - 5));
      ctx.setLineDash([]);
    }
    if (selected.has(keyOf(entity.id))) {
      ctx.strokeStyle = '#fff';
      ctx.lineWidth = 2;
      ctx.strokeRect(x - 1, y - 1, size + 2, size + 2);
    }
    if (entity.hp < entity.max_hp) {
      drawHealthBar(x + 2, y - 5, Math.max(3, size - 4), entity.hp / entity.max_hp);
    }
    if (tile >= 8) {
      ctx.fillStyle = 'rgba(255,255,255,0.85)';
      ctx.font = `${Math.min(12, Math.max(8, tile * 0.7))}px "Segoe UI", sans-serif`;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(entity.name[0], x + size / 2, y + size / 2);
    }
  }

  for (const entity of state.entities) {
    if (entity.kind !== 'unit') continue;
    const cx = view.ox + entity.x * tile;
    const cy = view.oy + entity.y * tile;
    const radius = Math.max(2, tile * 0.28);
    const color = ownerColor(entity.owner);
    ctx.beginPath();
    if (entity.role === 'Worker') {
      ctx.rect(cx - radius, cy - radius, radius * 2, radius * 2);
    } else {
      ctx.arc(cx, cy, radius, 0, Math.PI * 2);
    }
    ctx.fillStyle = color;
    ctx.fill();
    ctx.lineWidth = 1;
    ctx.strokeStyle = 'rgba(0,0,0,0.75)';
    ctx.stroke();
    if (selected.has(keyOf(entity.id))) {
      ctx.beginPath();
      ctx.arc(cx, cy, radius + 2.5, 0, Math.PI * 2);
      ctx.strokeStyle = '#fff';
      ctx.lineWidth = 2;
      ctx.stroke();
    }
    if (entity.hp < entity.max_hp) {
      drawHealthBar(cx - radius, cy - radius - 5, radius * 2, entity.hp / entity.max_hp);
    }
  }
}

function drawHealthBar(x, y, width, ratio) {
  ratio = Math.max(0, Math.min(1, ratio));
  const height = Math.max(2, Math.min(4, width * 0.16));
  ctx.fillStyle = 'rgba(0,0,0,0.75)';
  ctx.fillRect(x, y, width, height);
  ctx.fillStyle = ratio > 0.6 ? '#51cf66' : ratio > 0.3 ? '#fcc419' : '#ff6b6b';
  ctx.fillRect(x, y, width * ratio, height);
}

function drawSelectionBox() {
  if (!drag || !drag.moved) return;
  const x = Math.min(drag.x0, drag.x1);
  const y = Math.min(drag.y0, drag.y1);
  const width = Math.abs(drag.x1 - drag.x0);
  const height = Math.abs(drag.y1 - drag.y0);
  ctx.fillStyle = 'rgba(74,163,255,0.14)';
  ctx.fillRect(x, y, width, height);
  ctx.strokeStyle = '#4aa3ff';
  ctx.lineWidth = 1;
  ctx.strokeRect(x + 0.5, y + 0.5, width, height);
}

function drawBuildGhost() {
  if (!buildDef || !hover || !state) return;
  const footprint = buildDef.footprint;
  const origin = buildingOrigin(hover, footprint);
  const ok = canPlace(origin, footprint);
  const x = view.ox + origin.x * view.tile;
  const y = view.oy + origin.y * view.tile;
  const size = footprint * view.tile;
  ctx.fillStyle = ok ? 'rgba(81,207,102,0.22)' : 'rgba(255,107,107,0.25)';
  ctx.fillRect(x, y, size, size);
  ctx.strokeStyle = ok ? '#51cf66' : '#ff6b6b';
  ctx.lineWidth = 2;
  ctx.strokeRect(x + 1, y + 1, Math.max(1, size - 2), Math.max(1, size - 2));
}

function drawHover() {
  if (!hover || buildDef) return;
  ctx.strokeStyle = 'rgba(255,255,255,0.28)';
  ctx.lineWidth = 1;
  ctx.strokeRect(
    view.ox + hover.x * view.tile + 0.5,
    view.oy + hover.y * view.tile + 0.5,
    view.tile,
    view.tile,
  );
}

// ---------------------------------------------------------------------------
// DOM panels
// ---------------------------------------------------------------------------

function updateAll() {
  updateHud();
  updateSelection();
  updateBuild();
  updateLog();
  updateHint();
  draw();
}

function updateHud() {
  if (!state) return;
  const player = state.player;
  document.getElementById('clock').textContent = formatTime(state.seconds);
  document.getElementById('outcome').textContent = outcomeText(state.outcome);
  const resources = RES_KEYS.map(
    (key) =>
      `<span class="res res-${key}"><b>${RES_LABELS[key]}</b>${player.resources[key] || 0}</span>`,
  );
  document.getElementById('hud').innerHTML =
    `<span class="epoch">${player.epoch_name}</span>` +
    resources.join('') +
    `<span class="pop">Pop ${player.population}/${player.pop_cap}</span>`;
}

function hpBar(ratio) {
  const clamped = Math.max(0, Math.min(1, ratio));
  const className = clamped < 0.33 ? 'bad' : clamped < 0.66 ? 'warn' : '';
  return `<div class="hpbar ${className}"><i style="width:${clamped * 100}%"></i></div>`;
}

function updateSelection() {
  const element = document.getElementById('selection');
  if (!state) {
    element.innerHTML = '<div class="hint-text">Connecting...</div>';
    return;
  }
  const entities = selectedEntities();
  if (entities.length === 0) {
    element.innerHTML =
      '<div class="hint-text">Left-click a unit or building. Drag for a box selection. Right-click to move, attack, gather, or set a rally point.</div>';
    return;
  }

  if (entities.length === 1 && entities[0].kind === 'building') {
    const building = entities[0];
    let html = `<div class="sel-title">${building.name}</div>`;
    html += `<div class="muted">${building.finished ? 'Ready' : 'Under construction'} - HP ${building.hp}/${building.max_hp}</div>`;
    html += hpBar(building.hp / building.max_hp);

    if (building.queue.length > 0) {
      html += '<div class="subtitle">Queue</div>';
      for (let index = 0; index < building.queue.length; index++) {
        const item = building.queue[index];
        html += `<div class="queue-row"><span>${item.name}</span><span class="muted">${(item.ticks_left / 20).toFixed(1)}s</span><button data-cancel="${index}">x</button></div>`;
      }
    }

    const definition = state.catalog.buildings.find((candidate) => candidate.id === building.def);
    if (building.finished && definition && definition.trains.length > 0) {
      html += '<div class="subtitle">Train</div><div class="train-grid">';
      for (const unitId of definition.trains) {
        const unit = state.catalog.units.find((candidate) => candidate.id === unitId);
        if (!unit) continue;
        const epochOk = state.player.epoch >= unit.epoch;
        const affordable = canAfford(unit.cost);
        const popOk = state.player.population + unit.pop <= state.player.pop_cap;
        const disabled = !(epochOk && affordable && popOk);
        html += `<button data-train="${unit.id}" ${disabled ? 'disabled' : ''}><span>${unit.name}</span><span class="cost">${costText(unit.cost)} - ${unit.pop}p</span></button>`;
      }
      html += '</div>';
    }
    element.innerHTML = html;
    return;
  }

  const units = entities.filter((entity) => entity.kind === 'unit');
  const counts = new Map();
  for (const unit of units) counts.set(unit.name, (counts.get(unit.name) || 0) + 1);
  let html = `<div class="sel-title">${units.length} selected</div>`;
  html += [...counts.entries()]
    .map(([name, count]) => `<div class="muted">${count} x ${name}</div>`)
    .join('');
  html +=
    '<div class="toolbar"><button data-action="stop">Stop</button><button data-action="hold">Hold</button></div>';
  element.innerHTML = html;
}

function updateBuild() {
  const element = document.getElementById('build');
  if (!state) {
    element.innerHTML = '';
    return;
  }
  let html = '';
  if (buildDef) {
    html += `<div class="hint-text">Placing ${buildDef.name} - click the map. Right-click cancels.</div>`;
  }
  html += '<div class="build-grid">';
  for (const building of state.catalog.buildings) {
    if (building.role === 'TownCenter') continue;
    const epochOk = state.player.epoch >= building.required_epoch;
    const affordable = canAfford(building.cost);
    const disabled = !(epochOk && affordable);
    const active = buildDef && buildDef.id === building.id ? ' active' : '';
    html += `<button data-build="${building.id}" class="${active}" ${disabled ? 'disabled' : ''}><span>${building.name}</span><span class="cost">${costText(building.cost)}</span></button>`;
  }
  html += '</div>';
  element.innerHTML = html;
}

function updateLog() {
  if (!state) return;
  const text = state.log.slice(-80).join('\n');
  if (text === lastLog) return;
  lastLog = text;
  const element = document.getElementById('log-lines');
  element.textContent = text;
  element.scrollTop = element.scrollHeight;
}

function updateHint() {
  const element = document.getElementById('hint');
  if (!state) {
    element.textContent = 'Connecting...';
    return;
  }
  if (buildDef) {
    element.textContent = `Placing ${buildDef.name} - click the map, right-click to cancel`;
    return;
  }
  if (hover) {
    const entity = entityAt(hover);
    if (entity) {
      const belonging = entity.owner === state.human ? 'yours' : 'enemy';
      element.textContent = `${entity.name} - ${belonging} - ${entity.hp}/${entity.max_hp} HP`;
      return;
    }
    const node = nodeAt(hover);
    if (node) {
      element.textContent = `${node.kind} deposit - ${node.amount} left`;
      return;
    }
  }
  element.textContent = 'Left-click select - drag box - right-click move / attack / gather';
}

// ---------------------------------------------------------------------------
// Wiring
// ---------------------------------------------------------------------------

document.getElementById('selection').addEventListener('click', (event) => {
  const button = event.target.closest('button');
  if (!button || button.disabled || !state) return;
  const building = selectedBuildings()[0];
  if (button.dataset.train && building) {
    sendCommand({ Train: { building: building.id, unit: Number(button.dataset.train) } });
  } else if (button.dataset.cancel && building) {
    sendCommand({
      CancelProduction: { building: building.id, index: Number(button.dataset.cancel) },
    });
  } else if (button.dataset.action === 'stop') {
    sendCommand({ Stop: { units: selectedUnits().map((unit) => unit.id) } });
  } else if (button.dataset.action === 'hold') {
    sendCommand({ Hold: { units: selectedUnits().map((unit) => unit.id) } });
  }
});

document.getElementById('build').addEventListener('click', (event) => {
  const button = event.target.closest('button[data-build]');
  if (!button || button.disabled || !state) return;
  const definition = state.catalog.buildings.find(
    (candidate) => candidate.id === Number(button.dataset.build),
  );
  if (!definition) return;
  buildDef = definition;
  updateBuild();
  updateHint();
  draw();
});

document.getElementById('advance').addEventListener('click', () => sendCommand('AdvanceEpoch'));
document.getElementById('restart').addEventListener('click', restartMatch);

canvas.addEventListener('mousedown', onMouseDown);
canvas.addEventListener('contextmenu', onContextMenu);
window.addEventListener('mousemove', onMouseMove);
window.addEventListener('mouseup', onMouseUp);
window.addEventListener('resize', resizeCanvas);
new ResizeObserver(resizeCanvas).observe(canvas.parentElement);

resizeCanvas();
pollState();
setInterval(pollState, 250);
