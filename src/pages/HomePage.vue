<template>
  <section class="dashboard">
    <header class="page-header">
      <div><p class="eyebrow">Bom te ver novamente</p><h1>Seus projetos</h1><p>Gerencie ambientes, processos e atalhos em um só lugar.</p></div>
      <button class="primary-action" type="button"><Plus :size="17" /> Novo projeto</button>
    </header>

    <div class="stats-grid">
      <article v-for="stat in stats" :key="stat.label" class="stat-card">
        <div class="stat-card__icon" :class="`stat-card__icon--${stat.tone}`"><component :is="stat.icon" :size="18" /></div>
        <div><span>{{ stat.label }}</span><strong>{{ stat.value }}</strong></div>
        <span class="stat-card__detail">{{ stat.detail }}</span>
      </article>
    </div>

    <div class="section-title"><div><h2>Projetos recentes</h2><p>Continue de onde parou</p></div><button type="button">Ver todos <ArrowRight :size="14" /></button></div>
    <div class="projects-grid">
      <article v-for="project in projects" :key="project.name" class="project-card">
        <div class="project-card__top">
          <div class="project-icon" :style="{ '--project-color': project.color }"><FolderCode :size="21" /></div>
          <button type="button" aria-label="Mais opções"><Ellipsis :size="18" /></button>
        </div>
        <h3>{{ project.name }}</h3><p>{{ project.path }}</p>
        <div class="project-card__meta"><span><i :style="{ background: project.color }"></i>{{ project.stack }}</span><span><Clock3 :size="13" />{{ project.time }}</span></div>
      </article>
      <button class="new-project-card" type="button"><span><Plus :size="20" /></span><strong>Adicionar projeto</strong><small>Selecione uma pasta local</small></button>
    </div>
  </section>
</template>
<script setup lang="ts">
import { Activity, ArrowRight, Clock3, Ellipsis, FolderCode, FolderKanban, Play, Plus } from "lucide-vue-next";

const stats = [
  { label: "Projetos", value: "3", detail: "1 ativo agora", tone: "blue", icon: FolderKanban },
  { label: "Processos ativos", value: "2", detail: "Funcionando normalmente", tone: "green", icon: Play },
  { label: "Uso de memória", value: "1.2 GB", detail: "de 16 GB disponíveis", tone: "purple", icon: Activity },
];
const projects = [
  { name: "LocalCodePilot", path: "~/Projetos/LocalCodePilot", stack: "Vue + Tauri", time: "Agora", color: "#4f8cff" },
  { name: "Taskflow API", path: "~/Projetos/taskflow-api", stack: "Laravel", time: "Ontem", color: "#ef6461" },
  { name: "Portfolio", path: "~/Projetos/portfolio", stack: "Nuxt", time: "3 dias", color: "#41b883" },
];
</script>
<style scoped>
.page-header { display: flex; align-items: flex-end; justify-content: space-between; gap: 24px; margin-bottom: 28px; }
.eyebrow { margin: 0 0 7px !important; color: var(--color-primary) !important; font-size: 11px !important; font-weight: 700; letter-spacing: .08em; text-transform: uppercase; }
.page-header h1 { margin: 0; font-size: clamp(26px, 3vw, 34px); letter-spacing: -.04em; }
.page-header p { margin: 8px 0 0; color: var(--color-text-secondary); font-size: 13px; }
.primary-action { height: 38px; display: flex; align-items: center; gap: 8px; padding: 0 15px; color: white; background: var(--color-primary); border: 0; border-radius: 8px; font: inherit; font-size: 12px; font-weight: 600; cursor: pointer; box-shadow: 0 7px 20px rgba(79, 140, 255, .2); }
.primary-action:hover { background: var(--color-primary-dark); }
.stats-grid { display: grid; grid-template-columns: repeat(3, 1fr); gap: 14px; }
.stat-card { display: flex; align-items: center; gap: 13px; min-height: 92px; padding: 18px; background: var(--color-surface); border: 1px solid var(--color-border); border-radius: var(--radius-md); }
.stat-card__icon { width: 38px; height: 38px; display: grid; place-items: center; flex: 0 0 auto; border-radius: 9px; }
.stat-card__icon--blue { color: #6d9eff; background: rgba(79,140,255,.12); }.stat-card__icon--green { color: #45d283; background: rgba(46,204,113,.11); }.stat-card__icon--purple { color: #aa84ff; background: rgba(145,95,255,.12); }
.stat-card div:nth-child(2) { display: flex; flex-direction: column; gap: 3px; }.stat-card span { color: var(--color-text-secondary); font-size: 11px; }.stat-card strong { font-size: 19px; letter-spacing: -.02em; }.stat-card__detail { margin-left: auto; align-self: flex-end; color: #697282 !important; font-size: 10px !important; }
.section-title { display: flex; align-items: flex-end; justify-content: space-between; margin: 34px 0 14px; }.section-title h2 { margin: 0; font-size: 15px; }.section-title p { margin: 5px 0 0; color: var(--color-text-secondary); font-size: 11px; }.section-title button { display: flex; align-items: center; gap: 6px; color: #8daeff; background: none; border: 0; font: inherit; font-size: 11px; cursor: pointer; }
.projects-grid { display: grid; grid-template-columns: repeat(4, minmax(0, 1fr)); gap: 14px; }
.project-card, .new-project-card { min-height: 178px; padding: 18px; background: var(--color-surface); border: 1px solid var(--color-border); border-radius: var(--radius-md); transition: transform .2s, border-color .2s; }.project-card:hover, .new-project-card:hover { transform: translateY(-2px); border-color: #3a4351; }
.project-card__top { display: flex; justify-content: space-between; align-items: flex-start; }.project-card__top button { width: 28px; height: 28px; display: grid; place-items: center; color: #707888; background: transparent; border: 0; border-radius: 6px; cursor: pointer; }.project-card__top button:hover { background: #242832; color: white; }
.project-icon { width: 40px; height: 40px; display: grid; place-items: center; color: var(--project-color); background: color-mix(in srgb, var(--project-color) 12%, transparent); border-radius: 9px; }.project-card h3 { margin: 17px 0 5px; font-size: 13px; }.project-card > p { overflow: hidden; margin: 0; color: #747d8d; font-family: ui-monospace, monospace; font-size: 9px; text-overflow: ellipsis; white-space: nowrap; }
.project-card__meta { display: flex; justify-content: space-between; align-items: center; margin-top: 20px; }.project-card__meta span { display: flex; align-items: center; gap: 5px; color: #848d9d; font-size: 9px; }.project-card__meta i { width: 6px; height: 6px; border-radius: 50%; }
.new-project-card { display: flex; flex-direction: column; align-items: center; justify-content: center; color: #858e9e; background: transparent; border-style: dashed; font: inherit; cursor: pointer; }.new-project-card > span { width: 34px; height: 34px; display: grid; place-items: center; margin-bottom: 10px; background: #1c2029; border-radius: 8px; }.new-project-card strong { color: #b4bbc7; font-size: 11px; }.new-project-card small { margin-top: 4px; font-size: 9px; }
@media (max-width: 1100px) { .projects-grid { grid-template-columns: repeat(2, 1fr); }.stat-card__detail { display: none; } }
@media (max-width: 620px) { .page-header { align-items: flex-start; flex-direction: column; }.stats-grid, .projects-grid { grid-template-columns: 1fr; }.primary-action { width: 100%; justify-content: center; }.stat-card__detail { display: block; } }
</style>
