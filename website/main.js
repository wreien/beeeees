'use strict';

const log = document.getElementById('log');
function write(message) {
  log.insertAdjacentHTML('afterbegin', '<p>' + message + '</p>');
}

const websocket = new WebSocket('ws://' + window.location.host + '/observe');
const canvas = document.getElementById('canvas');
const ctx = canvas.getContext('2d');

var world = null;
var tile_size;

function resize() {
  const th = canvas.height / world.height;
  const tw = canvas.width / world.width;
  tile_size = Math.min(th, tw);
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
  for (var row = 0; row < world.width; ++row) {
    for (var col = 0; col < world.height; ++col) {
      ctx.fillStyle = tile_colour(world.map[row + col * world.width]);
      ctx.fillRect(row * tile_size, col * tile_size, tile_size, tile_size);
    }
  }
  ctx.restore();
}

function draw_flowers(flowers) {
  ctx.save();
  ctx.fillStyle = 'rgba(255, 255, 0, 0.5)';
  for (const f of flowers) {
    const { x, y } = f.position;
    ctx.fillRect(x * tile_size, y * tile_size, tile_size, tile_size);
  }
  ctx.restore();
}

function draw_bees(bees) {
  ctx.save();
  ctx.fillStyle = 'rgba(180, 0, 0, 0.7)';
  ctx.strokeStyle = 'black';
  for (const b of bees) {
    const x = b.position.x * tile_size + tile_size / 2;
    const y = b.position.y * tile_size + tile_size / 2;
    const jitter_x = (Math.random() - 0.5) * tile_size * 0.7;
    const jitter_y = (Math.random() - 0.5) * tile_size * 0.7;
    const radius = (0.1 + 0.05 * Math.random()) * tile_size;
    ctx.beginPath();
    ctx.arc(x + jitter_x, y + jitter_y, radius, 0, 2 * Math.PI);
    ctx.fill();
    ctx.stroke();
  }
  ctx.restore();
}

function draw(data) {
  ctx.clearRect(0, 0, canvas.width, canvas.height);
  draw_map();
  draw_flowers(data.flowers);
  draw_bees(data.bees);
}

websocket.onopen = () => write('CONNECTED');
websocket.onclose = () => write('DISCONNECTED');
websocket.onerror = e => write('<span class="error">ERROR:</span> ' + e.data);

websocket.onmessage = e => {
  const packet = JSON.parse(e.data);
  switch (packet.type) {
    case 'registration': 
      world = packet.world; 
      resize();
      break;
    case 'update': 
      draw(packet.data); 
      break;

    case 'done': write('<span>Received "done"</span>'); break;
    case 'warning': write('<span class="warning">WARNING:</span> ' + packet.msg); break;
    case 'error': write('<span class="error">ERROR:</span> ' + packet.msg); break;
    default: write('<span class="error">ERROR:</span> Unknown packet: ' + packet);
  }
}