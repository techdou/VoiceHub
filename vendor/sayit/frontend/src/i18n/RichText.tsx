/**
 * 把 `**加粗**` 标记渲染成 <strong>。
 *
 * 为什么需要它：有些说明句里有强调（"热词只提高**命中概率**"），原来是把句子
 * 拆成几段 JSX、中间夹 `<span className="font-semibold">`。一旦要翻译，这种写法
 * 就要求中英**语序一致**才拼得对 —— 而语序恰恰是最先不一致的东西。
 *
 * 改成让译文自己带标记：强调落在哪里由译者决定，代码不关心。
 * 只支持加粗一种标记，够用就不要再往上加（Markdown 子集会越滚越大）。
 */
import type { ReactNode } from 'react'

export function RichText({ text, strongClassName = 'font-semibold' }: {
  text: string
  strongClassName?: string
}) {
  const parts = text.split('**')
  return (
    <>
      {parts.map((part, index): ReactNode => (
        // 奇数段落在一对 ** 之间 —— split 的结果天然交替
        index % 2 === 1
          ? <span key={index} className={strongClassName}>{part}</span>
          : part
      ))}
    </>
  )
}
