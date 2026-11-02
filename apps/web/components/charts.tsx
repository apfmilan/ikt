"use client";

// lightweight-charts wrappers. The library is imported lazily inside effects so static
// export pre-rendering never touches the DOM.

import { useEffect, useRef } from "react";
import type { IChartApi, UTCTimestamp } from "lightweight-charts";
import { toNum } from "@/lib/format";
import type { ChartData } from "@/lib/types";

const COLORS = {
  text: "#8a8a8a",
  grid: "#121212",
  border: "#1f1f1f",
  gain: "#22c55e",
  loss: "#ef4444",
  accent: "#a3e635",
  muted: "#5c5c5c",
  forecast: "#7dd3fc",
};

const ts = (iso: string) => Math.floor(new Date(iso).getTime() / 1000) as UTCTimestamp;

function baseOptions(height: number, showTime: boolean) {
  return {
    height,
    autoSize: true,
    layout: {
      background: { color: "transparent" },
      textColor: COLORS.text,
      fontFamily: "ui-sans-serif, -apple-system, Segoe UI, sans-serif",
      fontSize: 11,
      attributionLogo: false,
    },
    grid: { vertLines: { visible: false }, horzLines: { color: COLORS.grid } },
    rightPriceScale: { borderVisible: false },
    timeScale: { borderVisible: false, timeVisible: showTime, secondsVisible: false },
    crosshair: { vertLine: { color: "#333", labelBackgroundColor: "#1a1a1a" }, horzLine: { color: "#333", labelBackgroundColor: "#1a1a1a" } },
    handleScale: { axisPressedMouseMove: false },
    localization: { locale: "hu-HU" },
  } as const;
}

/** Realized equity curve (HUF) with net deposits as a dashed reference line. */
export function EquityChart({
  points,
  height = 180,
}: {
  points: { t: string; equity_huf: string; net_deposits_huf: string }[];
  height?: number;
}) {
  const el = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!el.current || points.length < 2) return;
    let chart: IChartApi | undefined;
    let disposed = false;
    void import("lightweight-charts").then(({ createChart, AreaSeries, LineSeries, LineStyle }) => {
      if (disposed || !el.current) return;
      chart = createChart(el.current, {
        ...baseOptions(height, false),
        rightPriceScale: { visible: false },
        leftPriceScale: { visible: false },
        handleScroll: false,
        handleScale: false,
      });
      const first = toNum(points[0]?.equity_huf) ?? 0;
      const last = toNum(points[points.length - 1]?.equity_huf) ?? 0;
      const up = last >= first;
      const area = chart.addSeries(AreaSeries, {
        lineColor: up ? COLORS.gain : COLORS.loss,
        topColor: up ? "rgba(34,197,94,0.22)" : "rgba(239,68,68,0.22)",
        bottomColor: "rgba(0,0,0,0)",
        lineWidth: 2,
        priceLineVisible: false,
        lastValueVisible: false,
        crosshairMarkerRadius: 3,
      });
      area.setData(points.map((p) => ({ time: ts(p.t), value: toNum(p.equity_huf) ?? 0 })));
      const dep = chart.addSeries(LineSeries, {
        color: COLORS.muted,
        lineWidth: 1,
        lineStyle: LineStyle.Dashed,
        priceLineVisible: false,
        lastValueVisible: false,
        crosshairMarkerVisible: false,
      });
      dep.setData(points.map((p) => ({ time: ts(p.t), value: toNum(p.net_deposits_huf) ?? 0 })));
      chart.timeScale().fitContent();
    });
    return () => {
      disposed = true;
      chart?.remove();
    };
  }, [points, height]);

  if (points.length < 2) {
    return (
      <div className="flex items-center justify-center text-[13px] text-subtle" style={{ height }}>
        Még nincs elég adat a görbéhez.
      </div>
    );
  }
  return <div ref={el} style={{ height }} className="w-full" aria-label="Tőkegörbe" role="img" />;
}

/** Candles with entry/exit markers and position lines (avg price, stop, take profit). */
export function PriceChart({ data, height = 360 }: { data: ChartData; height?: number }) {
  const el = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!el.current || data.candles.length === 0) return;
    let chart: IChartApi | undefined;
    let disposed = false;
    void import("lightweight-charts").then(({ createChart, CandlestickSeries, HistogramSeries, createSeriesMarkers, LineStyle }) => {
      if (disposed || !el.current) return;
      chart = createChart(el.current, baseOptions(height, true));
      const candles = chart.addSeries(CandlestickSeries, {
        upColor: COLORS.gain,
        downColor: COLORS.loss,
        borderVisible: false,
        wickUpColor: COLORS.gain,
        wickDownColor: COLORS.loss,
      });
      const rows = [...data.candles].sort((a, b) => a.t.localeCompare(b.t));
      candles.setData(
        rows.map((c) => ({
          time: ts(c.t),
          open: toNum(c.o) ?? 0,
          high: toNum(c.h) ?? 0,
          low: toNum(c.l) ?? 0,
          close: toNum(c.c) ?? 0,
        })),
      );
      const vol = chart.addSeries(HistogramSeries, { priceScaleId: "vol", priceFormat: { type: "volume" }, lastValueVisible: false, priceLineVisible: false });
      chart.priceScale("vol").applyOptions({ scaleMargins: { top: 0.85, bottom: 0 } });
      vol.setData(
        rows.map((c) => ({
          time: ts(c.t),
          value: toNum(c.v) ?? 0,
          color: (toNum(c.c) ?? 0) >= (toNum(c.o) ?? 0) ? "rgba(34,197,94,0.25)" : "rgba(239,68,68,0.25)",
        })),
      );
      for (const l of data.lines) {
        const p = toNum(l.price);
        if (p === null) continue;
        candles.createPriceLine({
          price: p,
          color: l.kind === "stop" ? COLORS.loss : l.kind === "take_profit" ? COLORS.gain : COLORS.accent,
          lineWidth: 1,
          lineStyle: l.kind === "avg_price" ? LineStyle.Solid : LineStyle.Dashed,
          axisLabelVisible: true,
          title: l.kind === "stop" ? "Stop" : l.kind === "take_profit" ? "Cél" : "Átlagár",
        });
      }
      createSeriesMarkers(
        candles,
        [...data.markers]
          .sort((a, b) => a.t.localeCompare(b.t))
          .map((m) => ({
            time: ts(m.t),
            position: m.kind === "entry" ? ("belowBar" as const) : ("aboveBar" as const),
            shape: m.kind === "entry" ? ("arrowUp" as const) : ("arrowDown" as const),
            color: m.kind === "entry" ? COLORS.accent : m.kind === "stop" ? COLORS.loss : COLORS.forecast,
            text: m.text,
          })),
      );
      chart.timeScale().fitContent();
    });
    return () => {
      disposed = true;
      chart?.remove();
    };
  }, [data, height]);

  if (data.candles.length === 0) {
    return (
      <div className="flex items-center justify-center text-[13px] text-subtle" style={{ height }}>
        Nincs árfolyamadat.
      </div>
    );
  }
  return <div ref={el} style={{ height }} className="w-full" aria-label="Árfolyamgrafikon" role="img" />;
}

/** Daily realized net P&L bars. */
export function DailyPnlChart({ days, height = 160 }: { days: { date: string; net_pnl_huf: string }[]; height?: number }) {
  const el = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (!el.current || days.length === 0) return;
    let chart: IChartApi | undefined;
    let disposed = false;
    void import("lightweight-charts").then(({ createChart, HistogramSeries }) => {
      if (disposed || !el.current) return;
      chart = createChart(el.current, { ...baseOptions(height, false), handleScroll: false, handleScale: false });
      const s = chart.addSeries(HistogramSeries, { priceLineVisible: false, lastValueVisible: false });
      s.setData(
        [...days]
          .sort((a, b) => a.date.localeCompare(b.date))
          .map((d) => {
            const v = toNum(d.net_pnl_huf) ?? 0;
            return { time: ts(`${d.date}T12:00:00Z`), value: v, color: v >= 0 ? COLORS.gain : COLORS.loss };
          }),
      );
      chart.timeScale().fitContent();
    });
    return () => {
      disposed = true;
      chart?.remove();
    };
  }, [days, height]);
  if (days.length === 0) {
    return (
      <div className="flex items-center justify-center text-[13px] text-subtle" style={{ height }}>
        Még nincs lezárt nap.
      </div>
    );
  }
  return <div ref={el} style={{ height }} className="w-full" aria-label="Napi eredmény" role="img" />;
}
