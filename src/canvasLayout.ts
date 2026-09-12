/// 连线画布几何：设计尺寸 780×700，遥控器 152×620 居中（1:4.065 与
/// RC003 实物照片一致）。锚点坐标按 RC003 实物照片逐键重新标定
/// （黑色键面连通域分析：顶部 左电源/右语音、中部圆盘、左列 返回/主页/菜单、
/// 右列 音量±/TV），与 src/assets/rc003-remote.webp 像素级对齐。

import type { RemoteButtonId } from "./types";

export type Side = "left" | "right";
export type CanvasButtonId = RemoteButtonId;

export const CANVAS = {
  width: 780,
  height: 700,
  remote: { width: 152, height: 620 },
  card: { width: 250, height: 76 },
  /** 左列卡片左缘 x；右列 = width - cardWidth - margin。 */
  cardMargin: 8,
  /** 连线箭头与卡片边缘的间隙。 */
  arrowGap: 7,
} as const;

interface Placement {
  button: CanvasButtonId;
  side: Side;
  /** 遥控器上的锚点（0–1 相对坐标，按实物照片标定）。 */
  anchor: { x: number; y: number };
  /** 卡片中心 y（0–1 相对画布高）。 */
  targetY: number;
}

export const PLACEMENTS: Placement[] = [
  { button: "up", side: "left", anchor: { x: 0.502, y: 0.136 }, targetY: 0.22 },
  { button: "home", side: "left", anchor: { x: 0.294, y: 0.453 }, targetY: 0.46 },
  { button: "menu", side: "left", anchor: { x: 0.295, y: 0.546 }, targetY: 0.7 },
  { button: "ok", side: "right", anchor: { x: 0.502, y: 0.211 }, targetY: 0.34 },
  { button: "down", side: "right", anchor: { x: 0.502, y: 0.286 }, targetY: 0.46 },
  { button: "volume_up", side: "right", anchor: { x: 0.703, y: 0.363 }, targetY: 0.58 },
  { button: "volume_down", side: "right", anchor: { x: 0.703, y: 0.45 }, targetY: 0.7 },
];

export const VOICE_ANCHOR = { x: 0.759, y: 0.064 };
export const VOICE_TARGET_Y = 0.08;

export interface Point {
  x: number;
  y: number;
}

export function remoteOrigin(): Point {
  return {
    x: (CANVAS.width - CANVAS.remote.width) / 2,
    y: (CANVAS.height - CANVAS.remote.height) / 2,
  };
}

/** 遥控器相对锚点 → 画布绝对坐标。 */
export function remotePoint(anchor: Point): Point {
  const origin = remoteOrigin();
  return {
    x: origin.x + CANVAS.remote.width * anchor.x,
    y: origin.y + CANVAS.remote.height * anchor.y,
  };
}

export interface CardBox {
  button: CanvasButtonId | "voice";
  side: Side;
  /** 左上角。 */
  x: number;
  y: number;
  width: number;
  height: number;
  /** 靠遥控器一侧的边缘中点（连线终点）。 */
  edge: Point;
}

export function cardBox(side: Side, targetY: number, button: CanvasButtonId | "voice"): CardBox {
  const { width, height } = CANVAS.card;
  const x =
    side === "left"
      ? CANVAS.cardMargin
      : CANVAS.width - CANVAS.card.width - CANVAS.cardMargin;
  const y = CANVAS.height * targetY - height / 2;
  const edge = {
    x: side === "left" ? x + width : x,
    y: CANVAS.height * targetY,
  };
  return { button, side, x, y, width, height, edge };
}

export interface Link {
  button: CanvasButtonId | "voice";
  /** 贝塞尔路径 d。 */
  path: string;
  /** 箭头三点。 */
  arrow: [Point, Point, Point];
  /** 线起点（锚点，活动时画圆点）。 */
  start: Point;
}

/** 三次贝塞尔连线（控制点水平外扩，方向由侧决定）。 */
export function linkFor(start: Point, end: Point, side: Side): { c1: Point; c2: Point } {
  const direction = side === "left" ? -1 : 1;
  const distance = Math.min(70, Math.max(34, Math.abs(end.x - start.x) * 0.58));
  const endpointDistance = Math.min(42, Math.max(24, distance * 0.6));
  return {
    c1: { x: start.x + direction * distance, y: start.y },
    c2: { x: end.x - direction * endpointDistance, y: end.y },
  };
}

function buildLink(
  button: CanvasButtonId | "voice",
  anchor: Point,
  side: Side,
  targetY: number,
): Link {
  const start = remotePoint(anchor);
  const box = cardBox(side, targetY, button);
  const tip = {
    x: box.edge.x - (side === "left" ? -CANVAS.arrowGap : -CANVAS.arrowGap) * 0,
    y: box.edge.y,
  };
  // 箭头尖在卡片外侧 gap 处。
  tip.x = box.edge.x + (side === "left" ? -CANVAS.arrowGap : CANVAS.arrowGap);
  const { c1, c2 } = linkFor(start, tip, side);
  const direction = side === "left" ? -1 : 1;
  const arrow: [Point, Point, Point] = [
    tip,
    { x: tip.x - direction * 6, y: tip.y - 4 },
    { x: tip.x - direction * 6, y: tip.y + 4 },
  ];
  const d =
    `M ${start.x.toFixed(1)} ${start.y.toFixed(1)} ` +
    `C ${c1.x.toFixed(1)} ${c1.y.toFixed(1)}, ${c2.x.toFixed(1)} ${c2.y.toFixed(1)}, ` +
    `${tip.x.toFixed(1)} ${tip.y.toFixed(1)}`;
  return { button, path: d, arrow, start };
}

export function allCards(): CardBox[] {
  const cards = PLACEMENTS.map((p) => cardBox(p.side, p.targetY, p.button));
  cards.push(cardBox("right", VOICE_TARGET_Y, "voice"));
  return cards;
}

export function allLinks(): Link[] {
  const links = PLACEMENTS.map((p) => buildLink(p.button, p.anchor, p.side, p.targetY));
  links.push(buildLink("voice", VOICE_ANCHOR, "right", VOICE_TARGET_Y));
  return links;
}

/** 画布整体缩放：容器更窄时等比缩小（transform 用）。 */
export function canvasScale(containerWidth: number): number {
  return Math.min(1, containerWidth / CANVAS.width);
}
