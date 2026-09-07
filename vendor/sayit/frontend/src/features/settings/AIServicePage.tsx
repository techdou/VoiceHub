// AI 服务配置页面

import AIProviderSection from './AIProviderSection'
import { useT } from '@/i18n/useT'

export default function AIServicePage() {
  const t = useT()
  return (
    <div className="mx-auto max-w-4xl">
      <h1 className="mb-2 text-2xl font-bold">{t('nav.aiService')}</h1>
      <p className="mb-6 text-sm text-muted-foreground">
        {t('aiService.subtitle')}
      </p>
      <div className="space-y-6">
        <AIProviderSection />
      </div>
    </div>
  )
}
