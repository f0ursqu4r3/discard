import {
  computed,
  inject,
  onMounted,
  onUnmounted,
  provide,
  ref,
  type ComputedRef,
  type InjectionKey,
} from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'

export interface AppearancePreferences {
  miniProjectRail: boolean
  messageDensity: 'comfortable' | 'compact'
  showMessageAvatars: boolean
}

export const defaultAppearance: Readonly<AppearancePreferences> = Object.freeze({
  miniProjectRail: false,
  messageDensity: 'comfortable',
  showMessageAvatars: true,
})

const appearanceKey: InjectionKey<ComputedRef<Readonly<AppearancePreferences>>> =
  Symbol('appearance')

export function provideAppearance(preferences: () => Readonly<AppearancePreferences>) {
  provide(appearanceKey, computed(preferences))
}

export function useAppearanceSettings(onError: (error: string) => void) {
  const preferences = ref<AppearancePreferences>({ ...defaultAppearance })
  provideAppearance(() => preferences.value)
  let disposed = false
  let request = 0
  let stop: UnlistenFn | undefined
  async function reload() {
    const current = ++request
    try {
      const value = await invoke<AppearancePreferences>('settings_get', { page: 'appearance' })
      if (!disposed && current === request) preferences.value = { ...defaultAppearance, ...value }
    } catch (error) {
      if (!disposed && current === request) onError(String(error))
    }
  }
  onMounted(async () => {
    try {
      const unlisten = await listen('discard://settings', reload)
      if (disposed) return unlisten()
      stop = unlisten
      await reload()
    } catch (error) {
      if (!disposed) onError(String(error))
    }
  })
  onUnmounted(() => {
    disposed = true
    stop?.()
  })
  return preferences
}

export function useAppearance() {
  return inject(
    appearanceKey,
    computed(() => defaultAppearance),
  )
}
