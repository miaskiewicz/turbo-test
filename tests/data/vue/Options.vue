<script lang="ts">
import { defineComponent } from 'vue';
export default defineComponent({
  data(): { message: string } { return { message: 'options work' }; },
});
</script>
<template><p class="options">{{ message }}</p></template>
