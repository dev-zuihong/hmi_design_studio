import { computed, ref } from 'vue'
import type { Environment } from './model'

const platform = ref('windows-x64')
export const pythonRuntimeId = ref('3.14.8-win_amd64')
export const isMacOS = computed(() => platform.value.startsWith('macos-'))
export const platformLabel = computed(() => ({ 'windows-x64': 'Windows x64', 'macos-arm64': 'macOS Apple Silicon', 'macos-x64': 'macOS Intel x64' }[platform.value] ?? platform.value))
export function applyEnvironment(environment: Environment) {
  if (environment.platform) platform.value = environment.platform
  if (environment.pythonRuntimeId) pythonRuntimeId.value = environment.pythonRuntimeId
}
