const examples = {
  laravel: { manifest: 'composer.json', command: 'php artisan serve', ready: 'Servidor Laravel iniciado · http://localhost:8000' },
  node: { manifest: 'package.json', command: 'npm run dev', ready: 'Vite pronto · http://localhost:5173' },
  rust: { manifest: 'Cargo.toml', command: 'cargo run', ready: 'Processo Rust iniciado' }
};

const buttons = [...document.querySelectorAll('.demo-project')];
const manifest = document.querySelector('#demo-manifest');
const command = document.querySelector('#demo-command');
const log = document.querySelector('#demo-log');
const state = document.querySelector('#demo-state');
const run = document.querySelector('#demo-run');
const reset = document.querySelector('#demo-reset');
let selected = 'laravel';
let running = false;
let timer;

function addLine(message) {
  const line = document.createElement('p');
  const prompt = document.createElement('span');
  prompt.textContent = '>';
  line.append(prompt, document.createTextNode(' ' + message));
  log.append(line);
  log.scrollTop = log.scrollHeight;
}

function stop() {
  clearTimeout(timer);
  running = false;
  state.textContent = 'PARADO';
  state.classList.remove('is-running', 'is-starting');
  run.innerHTML = 'Iniciar simulação <span aria-hidden="true">↗</span>';
}

function selectProject(key) {
  stop();
  selected = key;
  buttons.forEach(button => {
    const active = button.dataset.project === key;
    button.classList.toggle('active', active);
    button.setAttribute('aria-pressed', String(active));
  });
  manifest.textContent = examples[key].manifest;
  command.textContent = examples[key].command;
  log.replaceChildren();
  addLine(`${examples[key].manifest} encontrado. Projeto pronto.`);
}

buttons.forEach(button => button.addEventListener('click', () => selectProject(button.dataset.project)));

run.addEventListener('click', () => {
  if (running) {
    stop();
    addLine('Processo encerrado na simulação.');
    return;
  }
  running = true;
  state.textContent = 'INICIANDO';
  state.classList.add('is-starting');
  run.innerHTML = 'Parar simulação <span aria-hidden="true">■</span>';
  addLine(`$ ${examples[selected].command}`);
  timer = setTimeout(() => {
    state.textContent = 'EXECUTANDO';
    state.classList.remove('is-starting');
    state.classList.add('is-running');
    addLine(examples[selected].ready);
  }, 650);
});

reset.addEventListener('click', () => selectProject(selected));
