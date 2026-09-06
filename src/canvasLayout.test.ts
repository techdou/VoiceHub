import { describe, expect, it } from "vitest";
import {
  allCards,
  allLinks,
  CANVAS,
  canvasScale,
  cardBox,
  PLACEMENTS,
  remotePoint,
} from "./canvasLayout";

describe("canvas layout geometry", () => {
  it("places every button card inside the canvas", () => {
    for (const placement of PLACEMENTS) {
      const box = cardBox(placement.side, placement.targetY, placement.button);
      expect(box.x, `${placement.button} x`).toBeGreaterThanOrEqual(0);
      expect(box.y, `${placement.button} y`).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width, `${placement.button} right`).toBeLessThanOrEqual(CANVAS.width);
      expect(box.y + box.height, `${placement.button} bottom`).toBeLessThanOrEqual(
        CANVAS.height + 0.5,
      );
    }
  });

  it("voice card fits at the top of the right column", () => {
    const voice = cardBox("right", 0.07, "voice");
    expect(voice.y).toBeGreaterThanOrEqual(0);
    expect(voice.y + voice.height).toBeLessThanOrEqual(CANVAS.height);
  });

  it("anchors map into the remote rectangle", () => {
    const origin = remotePoint({ x: 0, y: 0 });
    const corner = remotePoint({ x: 1, y: 1 });
    expect(origin.x).toBe((CANVAS.width - CANVAS.remote.width) / 2);
    expect(corner.x - origin.x).toBeCloseTo(CANVAS.remote.width);
    expect(corner.y - origin.y).toBeCloseTo(CANVAS.remote.height);
  });

  it("links start on the remote and point toward their card side", () => {
    for (const link of allLinks()) {
      const box = allCards().find((card) => card.button === link.button)!;
      // 起点：遥控器横向范围内。
      expect(link.start.x).toBeGreaterThan(CANVAS.width / 2 - 130);
      expect(link.start.x).toBeLessThan(CANVAS.width / 2 + 130);
      // 箭头尖在卡片边缘外侧 gap 处。
      const side = box.side;
      expect(Math.abs(link.arrow[0].x - box.edge.x)).toBeCloseTo(CANVAS.arrowGap, 5);
      // 路径字符串完整。
      expect(link.path).toMatch(/^M [\d.]+ [\d.]+ C /);
      // 箭头指向卡片：左列尖在左（x 最小），右列尖在右（x 最大）。
      if (box.side === "left") {
        expect(link.arrow[0].x).toBeLessThan(link.arrow[1].x);
      } else {
        expect(link.arrow[0].x).toBeGreaterThan(link.arrow[1].x);
      }
    }
  });

  it("left and right columns do not overlap the remote", () => {
    const remoteLeft = (CANVAS.width - CANVAS.remote.width) / 2;
    const remoteRight = remoteLeft + CANVAS.remote.width;
    for (const box of allCards()) {
      if (box.side === "left") {
        expect(box.x + box.width).toBeLessThanOrEqual(remoteLeft + 2);
      } else {
        expect(box.x).toBeGreaterThanOrEqual(remoteRight - 2);
      }
    }
  });

  it("scale shrinks below design width and caps at 1", () => {
    expect(canvasScale(CANVAS.width)).toBe(1);
    expect(canvasScale(2000)).toBe(1);
    expect(canvasScale(CANVAS.width / 2)).toBeCloseTo(0.5);
  });
});
