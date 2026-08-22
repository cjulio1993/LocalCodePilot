<template>
    <div class="base-card">
        <slot></slot>
    </div>
</template>

<script setup lang="ts">

</script>

<style scoped>
.base-card{
    background: var(--color-surface);
    border: 1px solid var(--color-border);
    border-radius: var(--radius-md);
    padding: var(--space-3);
    box-shadow: 0 2px 12px 0 rgba(0, 0, 0, 0.1);
    transition:
    border-color 0.2s ease,
    box-shadow 0.2s ease;
    overflow: hidden;
    display: flex;
    flex-direction: column;
}
</style>