<script setup lang="ts">
import { computed } from 'vue'
import DOMPurify from 'dompurify'
const props = defineProps<{ html: string }>()
const safe = computed(() => DOMPurify.sanitize(props.html))
</script>
<template><div v-html="safe" /></template>
