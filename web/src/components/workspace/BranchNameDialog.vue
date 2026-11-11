<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { toast } from 'vue-sonner'
import { createGitBranch, renameGitBranch } from '@/bridge/api'
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
  DialogDescription,
  DialogFooter,
} from '../ui/dialog'
import { Input } from '../ui/input'
import { Field, FieldError, FieldGroup, FieldLabel } from '../ui/field'
import { Button } from '../ui/button'

/**
 * Ask for a branch name. `rename` renames the branch `branch`. `create` starts a new branch in the working tree
 * `worktree`. The app checks the name with Git and says why it refuses: the dialog shows that reason and stays open.
 */
const open = defineModel<boolean>('open', { default: false })
const props = defineProps<{
  mode: 'rename' | 'create'
  projectId: string
  repository: string
  /** The branch to rename. */
  branch?: string
  /** The working tree that gets the new branch. */
  worktree?: string
  /** The branch the working tree holds now, for the description. */
  from?: string
}>()
const draft = ref('')
const busy = ref(false)
const error = ref<string | null>(null)
const trimmed = computed(() => draft.value.trim())
const id = computed(() => `${props.mode}-branch`)
watch(
  open,
  (value) => {
    if (!value) return
    draft.value = props.mode === 'rename' ? (props.branch ?? '') : ''
    error.value = null
  },
  { immediate: true },
)
watch(draft, () => (error.value = null))
// Focus the field. A rename starts with the current name selected, so typing replaces it.
function focusField(event: Event) {
  event.preventDefault()
  const field = document.getElementById(`${id.value}-name`) as HTMLInputElement | null
  field?.focus()
  field?.select()
}
async function submit() {
  if (busy.value || !trimmed.value) return
  busy.value = true
  error.value = null
  try {
    const name =
      props.mode === 'rename'
        ? await renameGitBranch(props.projectId, props.repository, props.branch ?? '', draft.value)
        : await createGitBranch(
            props.projectId,
            props.repository,
            props.worktree ?? '',
            draft.value,
          )
    toast.success(props.mode === 'rename' ? `Renamed to ${name}` : `Created ${name}`)
    open.value = false
  } catch (failure) {
    // The dialog stays open with the name intact, so the user can fix it.
    error.value = String(failure)
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
      :data-ui-id="`${id}-dialog`"
      :show-close-button="!busy"
      class="sm:max-w-md"
      @open-auto-focus="focusField"
    >
      <DialogHeader>
        <DialogTitle>{{ mode === 'rename' ? 'Rename branch' : 'Create branch' }}</DialogTitle>
        <DialogDescription v-if="mode === 'rename'">
          Git renames
          <span class="font-mono">{{ branch }}</span>
          . The channel keeps its messages.
        </DialogDescription>
        <DialogDescription v-else>
          The new branch starts at the current commit of
          <span class="font-mono">{{ from }}</span>
          . Uncommitted changes stay in the working tree.
        </DialogDescription>
      </DialogHeader>
      <form :id="`${id}-form`" class="flex flex-col gap-3" @submit.prevent="submit">
        <FieldGroup>
          <Field :data-invalid="error ? 'true' : undefined">
            <FieldLabel :for="`${id}-name`">Branch name</FieldLabel>
            <Input
              :id="`${id}-name`"
              v-model="draft"
              :data-ui-id="`${id}-input`"
              autocomplete="off"
              autocapitalize="off"
              spellcheck="false"
              class="font-mono"
              :aria-invalid="error ? 'true' : undefined"
              :disabled="busy"
              required
            />
            <FieldError v-if="error" :data-ui-id="`${id}-error`" role="alert">
              {{ error }}
            </FieldError>
          </Field>
        </FieldGroup>
      </form>
      <DialogFooter>
        <Button variant="ghost" :disabled="busy" @click="open = false">Cancel</Button>
        <Button
          type="submit"
          :form="`${id}-form`"
          :data-ui-id="`${id}-submit`"
          :disabled="busy || !trimmed"
        >
          {{
            busy
              ? mode === 'rename'
                ? 'Renaming…'
                : 'Creating…'
              : mode === 'rename'
                ? 'Rename Branch'
                : 'Create Branch'
          }}
        </Button>
      </DialogFooter>
    </DialogContent>
  </Dialog>
</template>
