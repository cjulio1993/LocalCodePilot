<template>
  <div class="app-shell">
    <aside class="sidebar" :class="{ 'sidebar--open': menuOpen }">
      <div class="brand">
        <div class="brand__mark" aria-hidden="true"><TerminalSquare :size="20" /></div>
        <div><strong>LocalCodePilot</strong><span>Workspace manager</span></div>
      </div>

      <nav class="navigation" aria-label="Navegação principal">
        <span class="navigation__label">Workspace</span>
        <RouterLink class="navigation__item" to="/" @click="menuOpen = false">
          <LayoutDashboard :size="18" /> Visão geral
        </RouterLink>
        <button class="navigation__item" type="button">
          <FolderKanban :size="18" /> Projetos <span class="navigation__count">3</span>
        </button>
        <button class="navigation__item" type="button"><Terminal :size="18" /> Processos</button>
        <span class="navigation__label navigation__label--spaced">Sistema</span>
        <button class="navigation__item" type="button"><Puzzle :size="18" /> Plugins</button>
        <button class="navigation__item" type="button"><Settings :size="18" /> Configurações</button>
      </nav>

      <div class="sidebar__footer">
        <span class="status-dot" aria-hidden="true"></span>
        <div><strong>Ambiente local</strong><span>Todos os serviços online</span></div>
      </div>
    </aside>

    <button v-if="menuOpen" class="sidebar-backdrop" type="button" aria-label="Fechar menu" @click="menuOpen = false"></button>

    <div class="app-main">
      <header class="topbar">
        <button class="icon-button menu-button" type="button" aria-label="Abrir menu" @click="menuOpen = true"><Menu :size="20" /></button>
        <div class="topbar__context"><span>Workspace</span><ChevronRight :size="14" /><strong>Visão geral</strong></div>
        <div class="topbar__actions">
          <button class="command-search" type="button"><Search :size="16" /><span>Buscar projeto...</span><kbd>⌘ K</kbd></button>
          <button class="icon-button" type="button" aria-label="Notificações"><Bell :size="18" /><span class="notification-dot"></span></button>
          <div class="avatar" title="Perfil de usuário">JC</div>
        </div>
      </header>
      <main class="content"><RouterView /></main>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref } from "vue";
import { Bell, ChevronRight, FolderKanban, LayoutDashboard, Menu, Puzzle, Search, Settings, Terminal, TerminalSquare } from "lucide-vue-next";

const menuOpen = ref(false);
</script>

<style scoped>
.app-shell { min-height: 100vh; background: var(--color-background); color: var(--color-text); }
.sidebar { position: fixed; inset: 0 auto 0 0; z-index: 30; width: 248px; display: flex; flex-direction: column; padding: 20px 14px; background: #12151c; border-right: 1px solid var(--color-border); }
.brand { display: flex; align-items: center; gap: 12px; padding: 0 8px 24px; }
.brand__mark { width: 36px; height: 36px; display: grid; place-items: center; color: white; background: linear-gradient(145deg, #5b8cff, #7a5cff); border-radius: 10px; box-shadow: 0 6px 20px rgba(79, 140, 255, .24); }
.brand div:last-child, .sidebar__footer div { display: flex; flex-direction: column; min-width: 0; }
.brand strong { font-size: 14px; letter-spacing: -.01em; }
.brand span, .sidebar__footer span { margin-top: 2px; color: var(--color-text-secondary); font-size: 11px; }
.navigation { display: flex; flex-direction: column; gap: 4px; }
.navigation__label { padding: 0 10px 7px; color: #6f7787; font-size: 10px; font-weight: 700; letter-spacing: .1em; text-transform: uppercase; }
.navigation__label--spaced { margin-top: 20px; }
.navigation__item { width: 100%; height: 42px; display: flex; align-items: center; gap: 11px; padding: 0 11px; color: var(--color-text-secondary); background: transparent; border: 0; border-radius: 8px; font: inherit; font-size: 13px; text-decoration: none; cursor: pointer; transition: .18s ease; }
.navigation__item:hover { color: var(--color-text); background: rgba(255,255,255,.045); }
.navigation__item.router-link-active { color: #fff; background: rgba(79, 140, 255, .13); box-shadow: inset 3px 0 var(--color-primary); }
.navigation__count { margin-left: auto; min-width: 20px; padding: 2px 6px; color: #8991a1; background: #20242e; border-radius: 10px; font-size: 10px; }
.sidebar__footer { margin-top: auto; display: flex; align-items: center; gap: 10px; padding: 14px 10px 2px; border-top: 1px solid var(--color-border); }
.sidebar__footer strong { font-size: 11px; font-weight: 600; }
.status-dot { width: 8px; height: 8px; flex: 0 0 auto; background: var(--color-success); border-radius: 50%; box-shadow: 0 0 0 4px rgba(46, 204, 113, .1); }
.app-main { min-height: 100vh; margin-left: 248px; }
.topbar { height: 64px; position: sticky; top: 0; z-index: 20; display: flex; align-items: center; justify-content: space-between; padding: 0 28px; background: rgba(15, 17, 23, .88); border-bottom: 1px solid var(--color-border); backdrop-filter: blur(16px); }
.topbar__context, .topbar__actions { display: flex; align-items: center; }
.topbar__context { gap: 6px; color: #707889; font-size: 12px; }
.topbar__context strong { color: var(--color-text); font-weight: 500; }
.topbar__actions { gap: 10px; }
.command-search { width: 220px; height: 34px; display: flex; align-items: center; gap: 8px; padding: 0 9px; color: #737c8d; background: #151820; border: 1px solid var(--color-border); border-radius: 8px; font: inherit; font-size: 11px; cursor: pointer; }
.command-search kbd { margin-left: auto; padding: 2px 5px; color: #7f8796; background: #20242d; border: 1px solid #303642; border-radius: 4px; font-family: inherit; font-size: 9px; }
.icon-button { position: relative; width: 34px; height: 34px; display: grid; place-items: center; color: var(--color-text-secondary); background: transparent; border: 1px solid transparent; border-radius: 8px; cursor: pointer; }
.icon-button:hover { color: white; background: #1b1f28; border-color: var(--color-border); }
.notification-dot { position: absolute; top: 7px; right: 7px; width: 5px; height: 5px; background: var(--color-primary); border: 1px solid var(--color-background); border-radius: 50%; }
.avatar { width: 32px; height: 32px; display: grid; place-items: center; margin-left: 2px; color: #dce7ff; background: #263552; border: 1px solid #38517b; border-radius: 9px; font-size: 10px; font-weight: 700; }
.content { width: 100%; max-width: 1440px; margin: 0 auto; padding: 36px 40px 48px; }
.menu-button, .sidebar-backdrop { display: none; }
@media (max-width: 800px) {
  .sidebar { transform: translateX(-100%); transition: transform .25s ease; }
  .sidebar--open { transform: translateX(0); }
  .sidebar-backdrop { position: fixed; inset: 0; z-index: 25; display: block; background: rgba(0,0,0,.58); border: 0; }
  .app-main { margin-left: 0; }
  .menu-button { display: grid; }
  .topbar { padding: 0 16px; }
  .topbar__context { margin-right: auto; margin-left: 8px; }
  .command-search { width: 34px; justify-content: center; }
  .command-search span, .command-search kbd { display: none; }
  .content { padding: 28px 20px 40px; }
}
@media (max-width: 480px) {
  .topbar__context span, .topbar__context svg { display: none; }
  .content { padding-inline: 16px; }
}
</style>
