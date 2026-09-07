import { formatElapsed, type RecordingPlayback } from './useRecordingPlayback'

const PLAYBACK_RATES = [0.75, 1, 1.5, 2, 2.5]

/**
 * 录音进度条 + 倍速。历史记录列表和「纠正识别」面板共用。
 *
 * 真正的 <input type=range> 是透明的、叠在自绘的细线和圆点上面 ——
 * 原生 range 的样式在 WebView2 里没法调到这个细度，但键盘可达性要靠它。
 */
export function AudioProgressBar({ playback, className = '' }: { playback: RecordingPlayback; className?: string }) {
  const { currentTime, duration, playbackRate, progress, seek, changeRate } = playback

  return (
    <div className={`flex items-center gap-2 ${className}`}>
      <span className="w-[72px] shrink-0 text-[11px] tabular-nums text-muted-foreground">
        {formatElapsed(currentTime)} / {formatElapsed(duration)}
      </span>
      <div className="relative h-3 min-w-0 flex-1 overflow-visible">
        <div className="absolute left-0 right-0 top-1/2 h-px -translate-y-1/2 bg-border" />
        <div className="absolute left-0 top-1/2 h-px -translate-y-1/2 bg-foreground" style={{ width: `${progress * 100}%` }} />
        <div
          className="absolute top-1/2 h-2 w-2 -translate-x-1/2 -translate-y-1/2 rounded-full border border-foreground bg-card shadow-sm"
          style={{ left: `${progress * 100}%` }}
        />
        <input
          type="range"
          min={0}
          max={Math.max(duration, 0.1)}
          step={0.1}
          value={Math.min(currentTime, duration || 0)}
          onChange={(e) => seek(Number(e.target.value))}
          className="absolute inset-0 h-full w-full cursor-pointer opacity-0"
        />
      </div>
      <div className="flex shrink-0 gap-0.5">
        {PLAYBACK_RATES.map((rate) => (
          <button
            key={rate}
            type="button"
            onClick={() => changeRate(rate)}
            className={`rounded px-1.5 py-0.5 text-[11px] transition-colors ${playbackRate === rate
              ? 'bg-foreground text-background font-medium'
              : 'text-muted-foreground hover:bg-accent'
              }`}
          >
            {rate}x
          </button>
        ))}
      </div>
    </div>
  )
}
