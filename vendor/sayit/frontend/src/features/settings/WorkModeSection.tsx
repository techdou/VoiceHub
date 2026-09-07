// 工作模式切换卡片

import { useEffect, useSyncExternalStore } from 'react'
import { Card, CardContent } from '@/components/ui/card'
import { Tooltip } from '@/components/ui/tooltip'
import { useConnectionStatus } from '@/hooks/useConnectionStatus'
import { getModeStatus, refreshModeStatus, subscribeModeStatus } from '@/stores/modeStatus'
// 这三个图标是在给模式**起名**，不是报状态，所以选的是"这个模式是什么"的最短表达：
// 本地在本机算 → Cpu，云 API 用你自己的云厂商 → Cloud，服务器模式连的是一台服务器 → Server。
// 比原来的 Monitor（显示器）/ Globe（地球）/ HardDrive（硬盘）都更贴。
//
// 左下角的引擎指示里，服务器模式用的是信号图标而不是 Server——那里报的是"此刻通不通"，
// 有一条真在跑的连接可报；这张卡不做这种断言（连接状态由卡组右上角那枚徽标负责）。
// 两处对本地 / 云 API 用同一个图标，对服务器模式刻意不同，因为它们说的不是同一件事。
import { Cpu, Cloud, Server, CheckCircle2, type LucideIcon } from 'lucide-react'
import type { WorkMode } from '@/services/transcription'
import type { TranslationKey } from '@/i18n'
import { useT } from '@/i18n/useT'

// 模块级常量只求值一次，所以存 key、渲染时才翻。
const modes: Array<{ value: WorkMode; labelKey: TranslationKey; descKey: TranslationKey; privacyKey: TranslationKey; icon: LucideIcon }> = [
  {
    value: 'local', labelKey: 'mode.local',
    descKey: 'workMode.local.desc',
    privacyKey: 'workMode.local.privacy',
    icon: Cpu,
  },
  {
    value: 'cloud_api', labelKey: 'mode.cloudApi',
    descKey: 'workMode.cloudApi.desc',
    privacyKey: 'workMode.cloudApi.privacy',
    icon: Cloud,
  },
  {
    value: 'server', labelKey: 'mode.server',
    descKey: 'workMode.server.desc',
    privacyKey: 'workMode.server.privacy',
    icon: Server,
  },
]

const statusConfig = {
  connected: { dot: 'bg-success', textKey: 'status.connected', bg: 'bg-success/10 text-success-strong' },
  connecting: { dot: 'bg-warning animate-pulse', textKey: 'status.connecting', bg: 'bg-warning/10 text-warning-strong' },
  disconnected: { dot: 'bg-muted-foreground', textKey: 'status.disconnected', bg: 'bg-muted text-muted-foreground' },
  error: { dot: 'bg-destructive', textKey: 'status.error', bg: 'bg-destructive/10 text-destructive-strong' },
} as const satisfies Record<string, { dot: string; textKey: TranslationKey; bg: string }>

interface Props {
  value: WorkMode
  onChange: (mode: WorkMode) => void
}

export default function WorkModeSection({ value, onChange }: Props) {
  const t = useT()
  const wsStatus = useConnectionStatus()
  const { ready, blockedReason } = useSyncExternalStore(subscribeModeStatus, getModeStatus)

  // 徽标反映真实就绪状态，所以本组件也要在挂载时拉一次（页面可能是直接深链进来的）
  useEffect(() => { void refreshModeStatus() }, [])

  /**
   * 状态徽标。
   *
   * 这里曾经是三个三元：服务器模式看真实连接状态，本地/云 API **一律**绿点 +「就绪」。
   * 结果是模型没下载、密钥没填也显示绿灯，而卡片正下方同时写着「模型尚未下载」——
   * 同屏自相矛盾。现在本地/云 API 的就绪判断来自 modeStatus（见该 store 的注释），
   * 未就绪时显示「待配置」并可点击滚到对应的配置卡。
   */
  const badge = value === 'server'
    ? { dot: statusConfig[wsStatus].dot, bg: statusConfig[wsStatus].bg, text: t(statusConfig[wsStatus].textKey), hint: '' }
    : ready === false
      ? {
        dot: 'bg-warning',
        text: t('workMode.badge.needsSetup'),
        bg: 'bg-warning/10 text-warning-strong',
        // blockedReason 仍是服务层给的中文串（P2-1 会 code 化），这里只保证外壳跟随语言
        hint: blockedReason ? t('workMode.hintBlocked', { reason: blockedReason }) : t('workMode.hintIncomplete'),
      }
      : ready === true
        ? { dot: 'bg-success', text: t('workMode.badge.ready'), bg: 'bg-success/10 text-success-strong', hint: '' }
        : { dot: 'bg-muted-foreground', text: t('workMode.badge.checking'), bg: 'bg-muted text-muted-foreground', hint: '' }

  const scrollToConfig = () => {
    document.getElementById('engine-config')?.scrollIntoView({ behavior: 'smooth', block: 'start' })
  }

  const badgeBody = (
    <>
      <span className={`inline-block h-1.5 w-1.5 shrink-0 rounded-full ${badge.dot}`} aria-hidden />
      {badge.text}
    </>
  )
  const badgeClass = `inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs font-medium ${badge.bg}`

  return (
    <Card>
      <CardContent className="p-6">
        <div className="mb-4 flex items-center justify-between gap-3">
          <h2 id="work-mode-heading" className="text-lg font-semibold">{t('workMode.title')}</h2>

          {badge.hint ? (
            <Tooltip variant="light" content={t('workMode.tooltipJump', { hint: badge.hint })}>
              <button
                type="button"
                onClick={scrollToConfig}
                className={`${badgeClass} transition-colors hover:bg-warning/20`}
              >
                {badgeBody}
              </button>
            </Tooltip>
          ) : (
            <span className={badgeClass} role="status">{badgeBody}</span>
          )}
        </div>

        <div
          role="radiogroup"
          aria-labelledby="work-mode-heading"
          className="grid gap-3 sm:grid-cols-3"
        >
          {modes.map((m) => {
            const isActive = value === m.value
            const Icon = m.icon
            return (
              <button
                key={m.value}
                type="button"
                role="radio"
                aria-checked={isActive}
                onClick={() => onChange(m.value)}
                className={`relative rounded-lg border p-4 text-left transition-colors ${isActive
                  ? 'border-primary bg-primary/5'
                  : 'border-border hover:bg-accent'
                  }`}
              >
                <Icon
                  className={`absolute right-3 top-3 h-5 w-5 transition-colors ${isActive ? 'text-primary' : 'text-muted-foreground'}`}
                  aria-hidden
                />
                {/* 选中态原来只有颜色（1px 边框 + 图标变色 + 5% 的淡底）。加一个对勾，
                    让"当前是哪个"不依赖颜色感知。
                    用 CheckCircle2 + success-strong 而不是裸 Check + primary，是为了和
                    「语音识别服务」卡片上的「使用中」标记完全一致 —— 两页都是在选一张卡，
                    同一个含义不该长成两个样子。别顺手把它统一回 primary。
                    图标保持 aria-hidden：选中状态已经由外层 role="radio" 的 aria-checked
                    如实报给读屏，再给图标加一个 label 只会让它念两遍。 */}
                <div className="flex items-center gap-1.5 pr-7 text-sm font-medium">
                  {isActive && <CheckCircle2 className="h-4 w-4 shrink-0 text-success-strong" aria-hidden />}
                  {t(m.labelKey)}
                </div>
                <div className="mt-1 text-xs text-muted-foreground">{t(m.descKey)}</div>
                <div className="mt-2 border-t border-border/50 pt-2 text-xs leading-relaxed text-muted-foreground">
                  {t(m.privacyKey)}
                </div>
              </button>
            )
          })}
        </div>
      </CardContent>
    </Card>
  )
}
