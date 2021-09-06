'use strict';

const log = document.getElementById('log');
function write(message) {
  log.insertAdjacentHTML('afterbegin', '<p>' + message + '</p>');
}

const websocket = new WebSocket('ws://' + window.location.host + '/observe');
const canvas = document.getElementById('canvas');
const ctx = canvas.getContext('2d');

const world = {
  tile_size: null,
  width: null,
  height: null,
  map: null,
  at: function (x, y) { return this.map[x + y * this.width] },
};

const players = {
  max: 0,
  data: new Map(),
};

const bees = new Map();    // ID -> bee data
const flowers = new Map(); // ID -> flower data

let last_tick = null;         // when the last tick occurred (ms)
let tick_length = null;       // number of milliseconds between ticks
let ticks_per_update = null;  // expected number of ticks between each game update

function resize() {
  const th = canvas.height / world.height;
  const tw = canvas.width / world.width;
  world.tile_size = Math.min(th, tw);
}

function init(new_world, new_tick_rate) {
  Object.assign(world, new_world);
  resize();

  players.max = world.map.filter(t => t === 'SpawnPoint').length;
  players.data.clear();

  tick_length = 20;  // 50Hz
  ticks_per_update = (new_tick_rate * 1000) / tick_length;
}

function make_ellipse(a, b, initial_t, t_step) {
  return {
    t: initial_t,
    d: t_step,
    x: 0.0,
    y: 0.0,
    update: function (p) {
      this.t += this.d * p;
      if (this.t > 2 * Math.PI) {
        this.t -= 2 * Math.PI;
      }
      this.x = a * world.tile_size * Math.cos(this.t);
      this.y = b * world.tile_size * Math.sin(this.t);
    },
  };
}

function update(data) {
  for (const h of data.hives) {
    if (!players.data.has(h.player)) {
      const colour = 360 * players.data.size / players.max;
      players.data.set(h.player, Object.assign({ colour, opacity: 0 }, h));
    }
  }

  flowers.forEach(v => v.next = null);
  for (const f of data.flowers) {
    if (!flowers.has(f.id)) {
      flowers.set(f.id, { curr: Object.assign({ opacity: 0 }, f), next: f });
    } else {
      flowers.get(f.id).next = f;
    }
  }

  bees.forEach(v => v.next = null);
  for (const b of data.bees) {
    if (!bees.has(b.id)) {
      const radius = (0.1 + 0.05 * Math.random()) * world.tile_size;
      const jitter = make_ellipse(
        Math.random() * 0.3 + 0.05,
        Math.random() * 0.3 + 0.05,
        Math.random() * 2 * Math.PI,
        (Math.random() + 1) * (Math.random() > 0.5 ? 1 : -1) * Math.PI,
      );
      const colour = players.data.get(b.player).colour;
      bees.set(b.id, { curr: Object.assign({ radius, jitter, colour }, b), next: b });
    } else {
      bees.get(b.id).next = b;
    }
  }
}

function animate(num_ticks) {
  const update_step = num_ticks / ticks_per_update;
  const time_step = num_ticks * tick_length / 1000;

  players.data.forEach(p => {
    if (p.opacity < 1.0) {
      p.opacity = Math.min(1.0, p.opacity + update_step);
    }
  });

  flowers.forEach((f, k, m) => {
    if (f.next === null) {
      if (f.curr.opacity > 0.0) {
        f.curr.opacity -= update_step;
      } else {
        m.delete(k);
      }
    } else {
      if (f.curr.opacity < 1.0) {
        f.curr.opacity = Math.min(0.7, f.curr.opacity + update_step);
      }
    }
  });

  bees.forEach((b, k, m) => {
    if (b.next === null) {
      m.delete(k);
    } else {
      b.curr.jitter.update(update_step);

      const cp = b.curr.position;
      const np = b.next.position;
      const off = update_step / 2;

      if (cp.x < np.x - off) {
        cp.x += update_step;
      } else if (cp.x > np.x + off) {
        cp.x -= update_step;
      }

      if (cp.y <= np.y - off) {
        cp.y += update_step;
      } else if (cp.y >= np.y + off) {
        cp.y -= update_step
      }
    }
  });
}

function tile_colour(tile) {
  switch (tile) {
    case 'Grass': return '#88FF88';
    case 'Garden': return '#00CC00';
    case 'Neutral': return '#666666';
    case 'Road': return '#AAAAAA';
    case 'Block': return 'brown';
    case 'SpawnPoint': return '#444444';
  }
}

function draw_map() {
  ctx.save();
  const s = world.tile_size;
  for (let row = 0; row < world.width; ++row) {
    for (let col = 0; col < world.height; ++col) {
      ctx.fillStyle = tile_colour(world.at(row, col));
      ctx.fillRect(row * s, col * s, s, s);
    }
  }
  ctx.restore();
}

function draw_flowers() {
  ctx.save();
  const s = world.tile_size;
  for (const { curr, _ } of flowers.values()) {
    const { x, y } = curr.position;
    ctx.fillStyle = `rgba(255, 255, 0, ${curr.opacity})`
    ctx.fillRect(x * s, y * s, s, s);
  }
  ctx.restore();
}

function draw_hives() {
  ctx.save();
  const s = world.tile_size;
  for (const p of players.data.values()) {
    const { x, y } = p.position;
    ctx.fillStyle = `hsla(${p.colour}, 100%, 50%, ${p.opacity})`;
    ctx.fillRect(x * s, y * s, s, s);
  }
  ctx.restore();
}

function draw_bees() {
  ctx.save();
  ctx.strokeStyle = 'black';
  for (const { curr, _ } of bees.values()) {
    const x = (curr.position.x + 0.5) * world.tile_size;
    const y = (curr.position.y + 0.5) * world.tile_size;
    ctx.fillStyle = `hsla(${curr.colour}, 100%, 50%, 0.7)`;
    ctx.beginPath();
    ctx.arc(x + curr.jitter.x, y + curr.jitter.y, curr.radius, 0, 2 * Math.PI);
    ctx.fill();
    ctx.stroke();
  }
  ctx.restore();
}

function draw() {
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  draw_map();
  draw_flowers();
  draw_hives();
  draw_bees();
}

function main(now) {
  window.requestAnimationFrame(main);
  const next_tick = last_tick + tick_length;

  if (now >= next_tick) {
    const time_since_tick = now - last_tick;
    const num_ticks = Math.floor(time_since_tick / tick_length);
    // TODO: refresh all state to 'next' values if num_ticks too large (sleeping?)
    last_tick += num_ticks * tick_length;
    animate(num_ticks);
    draw();
  }
}

websocket.onopen = () => write('CONNECTED');
websocket.onclose = () => write('DISCONNECTED');
websocket.onerror = e => write('<span class="error">ERROR:</span> ' + e.data);

websocket.onmessage = e => {
  const packet = JSON.parse(e.data);
  switch (packet.type) {
    case 'registration':
      init(packet.world, packet.tick_rate);
      break;
    case 'update':
      update(packet.data);
      if (last_tick === null) {
        last_tick = performance.now();
        main(last_tick);
      }
      break;

    case 'done': write('<span>Received "done"</span>'); break;
    case 'warning': write('<span class="warning">WARNING:</span> ' + packet.msg); break;
    case 'error': write('<span class="error">ERROR:</span> ' + packet.msg); break;
    default: write('<span class="error">ERROR:</span> Unknown packet: ' + packet);
  }
}