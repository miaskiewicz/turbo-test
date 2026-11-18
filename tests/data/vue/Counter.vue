<script setup lang="ts">
import { ref } from 'vue';
const props = defineProps<{ label: string }>();
const count = ref<number>(1);
function increment(): void { count.value += 1; }
</script>
<template><button class="counter" @click="increment">{{ props.label }} {{ count }}</button></template>
<style scoped>.counter { color: rgb(10, 20, 30); }</style>
