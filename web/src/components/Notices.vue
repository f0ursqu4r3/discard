<script setup lang="ts">
defineOptions({ name: "NoticeList" })
import type { BannersDto } from '../bridge/types'
import { confirmResolve, archiveConfirm, runAction } from '../bridge/api'
import { Alert } from './ui/alert'
import { Button } from './ui/button'
defineProps<{ banners: BannersDto; notice: string | null; archived: boolean }>()
const protectedClass = 'mx-4 mb-2 flex items-center gap-2 border-[var(--protected-border)] bg-[var(--protected-bg)] text-[var(--protected-fg)]'
const protectedButton = 'border-[var(--protected-border)] bg-[var(--protected-bg)] text-[var(--protected-fg)] hover:bg-[var(--protected-bg)] hover:text-[var(--protected-fg)] focus-visible:border-[var(--protected-border)] focus-visible:ring-[var(--protected-border)] aria-invalid:border-[var(--protected-border)] aria-invalid:ring-[var(--protected-border)] dark:aria-invalid:border-[var(--protected-border)] dark:aria-invalid:ring-[var(--protected-border)]'
</script>
<template>
  <Alert v-if="banners.confirm" data-protected="true" :class="protectedClass"><span class="flex-1">{{ banners.confirm }}</span><Button variant="outline" size="sm" :class="protectedButton" @click="confirmResolve(false)">Cancel</Button><Button variant="outline" size="sm" :class="protectedButton" @click="confirmResolve(true)">Confirm</Button></Alert>
  <Alert v-if="banners.archive || archived" data-protected="true" :class="protectedClass"><span class="flex-1 whitespace-pre-wrap">{{ banners.archive?.message || 'Channel archived' }}</span><template v-if="banners.archive"><Button variant="outline" size="sm" :class="protectedButton" @click="archiveConfirm(false)">Cancel</Button><Button variant="outline" size="sm" :class="protectedButton" @click="archiveConfirm(true)">Archive and delete</Button></template><Button v-else-if="archived" variant="outline" size="sm" :class="protectedButton" @click="runAction('relaunch')">Relaunch</Button></Alert>
  <Alert v-if="banners.reviewCount" data-protected="true" :class="protectedClass"><span class="flex-1">Plugin code is waiting for your review: {{ banners.reviewNames?.join(', ') || banners.reviewCount }}</span><Button variant="outline" size="sm" :class="protectedButton" @click="runAction('review_plugins')">Review</Button></Alert>
  <Alert v-if="banners.problemLines.length" data-protected="true" :class="protectedClass"><span class="flex-1">{{ banners.problemLines.length }} plugin {{ banners.problemLines.length === 1 ? 'problem' : 'problems' }}</span><Button variant="outline" size="sm" :class="protectedButton" @click="runAction('show_plugin_errors')">Details</Button></Alert>
  <Alert v-if="notice" class="mx-4 mb-2 border-warning text-foreground">{{ notice }}</Alert>
</template>
