<script setup lang="ts">
import { ref, watch } from 'vue'
import { Hash, LoaderCircle, Bot, GitBranch, Folder } from '@lucide/vue'
import { toast } from 'vue-sonner'
import { createChannel } from '@/bridge/api'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '../ui/dialog'
import { Field, FieldGroup, FieldLabel, FieldDescription, FieldError } from '../ui/field'
import { Input } from '../ui/input'
import { Button } from '../ui/button'
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '../ui/select'
const open = defineModel<boolean>('open', { default: false })
const props = defineProps<{
  project: string
  channelId: string
  agent: string
  provider: string
  hasGit: boolean
}>()
const emit = defineEmits<{ created: [id: string] }>()
const name = ref(''),
  mode = ref('worktree'),
  error = ref(''),
  busy = ref(false),
  source = ref('')
watch(open, (value) => {
  if (value) {
    name.value = ''
    error.value = ''
    mode.value = props.hasGit ? 'worktree' : 'main_checkout'
    source.value = props.channelId
  }
})
async function submit() {
  if (busy.value) return
  const value = name.value.trim().replace(/^#+/, '').toLowerCase()
  if (!value || value.length > 64 || !/^[\p{L}\p{N}_./-]+$/u.test(value)) {
    error.value = 'Use 1–64 letters, numbers, dashes, underscores, dots, or slashes.'
    return
  }
  busy.value = true
  error.value = ''
  try {
    const id = await createChannel(value, mode.value, source.value)
    open.value = false
    emit('created', id)
    toast.success(`Created #${value}`)
  } catch (cause) {
    error.value = String(cause)
  } finally {
    busy.value = false
  }
}
</script>
<template>
  <Dialog
    :open="open"
    @update:open="
      (value) => {
        if (!busy) open = value
      }
    "
  >
    <DialogContent
      data-ui-id="create-channel-dialog"
      class="max-h-[calc(100dvh-2rem)] overflow-y-auto sm:max-w-md"
      :show-close-button="!busy"
      @interact-outside="
        (event) => {
          if (busy) event.preventDefault()
        }
      "
      @escape-key-down="
        (event) => {
          if (busy) event.preventDefault()
        }
      "
    >
      <DialogHeader>
        <DialogTitle>Create channel</DialogTitle>
        <DialogDescription>{{ project }}</DialogDescription>
      </DialogHeader>
      <form
        id="create-channel-form"
        data-ui-id="create-channel-form"
        class="flex flex-col gap-5"
        @submit.prevent="submit"
      >
        <FieldGroup>
          <Field :data-invalid="!!error">
            <FieldLabel for="channel-name">Channel name</FieldLabel>
            <div class="relative">
              <Hash
                class="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground"
              />
              <Input
                id="channel-name"
                v-model="name"
                data-ui-id="channel-name-input"
                class="pl-9"
                placeholder="feat/session-isolation"
                autocomplete="off"
                :maxlength="64"
                :disabled="busy"
                :aria-invalid="!!error"
                :aria-describedby="error ? 'channel-error' : undefined"
              />
            </div>
          </Field>
          <Field>
            <FieldLabel for="channel-workspace">Workspace</FieldLabel>
            <Select v-model="mode" :disabled="busy">
              <SelectTrigger
                id="channel-workspace"
                data-ui-id="channel-workspace-select"
                class="w-full"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="worktree" :disabled="!hasGit">
                  <GitBranch class="size-4" />
                  New isolated worktree
                </SelectItem>
                <SelectItem value="main_checkout">
                  <Folder class="size-4" />
                  Project folder
                </SelectItem>
              </SelectContent>
            </Select>
            <FieldDescription>
              {{
                mode === 'worktree'
                  ? 'A new branch and worktree will be created when the agent starts work.'
                  : 'The agent will work directly in your project folder.'
              }}
            </FieldDescription>
          </Field>
          <Field>
            <FieldLabel>Agent</FieldLabel>
            <div
              data-ui-id="channel-agent"
              class="flex items-center gap-3 rounded-lg border bg-muted/40 p-3"
            >
              <span
                class="flex size-8 shrink-0 items-center justify-center rounded-full bg-primary/15 text-primary"
              >
                <Bot class="size-4" />
              </span>
              <div class="min-w-0">
                <p class="truncate text-sm font-medium">{{ agent }}</p>
                <p class="text-xs text-muted-foreground">
                  {{ provider }} · Uses this agent’s settings
                </p>
              </div>
            </div>
          </Field>
        </FieldGroup>
        <FieldError v-if="error" id="channel-error" role="alert">{{ error }}</FieldError>
      </form>
      <DialogFooter>
        <Button variant="ghost" :disabled="busy" @click="open = false">Cancel</Button>
        <Button
          type="submit"
          form="create-channel-form"
          data-ui-id="create-channel-submit"
          :disabled="busy || !name.trim()"
        >
          <LoaderCircle v-if="busy" class="motion-safe:animate-spin" />
          {{ busy ? 'Creating…' : 'Create channel' }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
