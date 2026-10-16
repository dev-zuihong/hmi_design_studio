import { inject, provide, type InjectionKey } from 'vue'

/** Opaque host-managed reference. Widgets never read firmware or trust a user-entered path. */
export interface RuntimeFileRef { id: string; name: string; size: number; sha256: string; path?: string; status?: string }
export interface RuntimeFileRequest { accept: string; value?: RuntimeFileRef | null; path?: string }
export interface RuntimeFileService { choose(request: RuntimeFileRequest): Promise<RuntimeFileRef | undefined> }
const runtimeFileService: InjectionKey<RuntimeFileService> = Symbol('runtime-file-service')
export function provideRuntimeFileService(service: RuntimeFileService) { provide(runtimeFileService, service) }
export function useRuntimeFileService() { return inject(runtimeFileService, undefined) }
