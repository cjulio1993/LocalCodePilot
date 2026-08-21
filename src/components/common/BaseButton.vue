<template>
    <button type="button"  class="base-button">
        <slot></slot>
    </button>
</template>

<script setup lang="ts">

</script>

<style scoped>

.base-button {
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-md);
    border: none;
    background-color: var(--color-primary);
    color: var(--color-text);
    transition: all 0.2s ease-in-out;
    user-select: none;
    display: inline-flex;
    align-items: center;
    justify-content: center;
    gap: var(--space-1);
    font: inherit;
    white-space: nowrap;
}

.base-button:hover {
    background-color: var(--color-primary-dark);
}

.base-button:active {
    transform: scale(0.98);
}
</style>