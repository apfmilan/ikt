"use client";

import { useState } from "react";
import { DecisionItem } from "@/components/decision";
import { PageHeader } from "@/components/page-header";
import { EmptyState, ErrorState, LoadingRows } from "@/components/states";
import { Card } from "@/components/ui/card";
import { Segmented } from "@/components/ui/segmented";
import { api } from "@/lib/api";
import { useResource } from "@/lib/live";

type Filter = "all" | "actions" | "vetoes";

export default function TimelinePage() {
  const [filter, setFilter] = useState<Filter>("all");
  const res = useResource(() => api.decisions(100), ["decision"]);
  const rows = (res.data ?? []).filter((d) =>
    filter === "all" ? true : filter === "actions" ? d.action === "enter" || d.action === "exit" : d.risk?.verdict === "veto",
  );

  return (
    <div className="animate-fade-up">
      <PageHeader title="Döntési napló" subtitle="Minden belépés, kilépés és tartózkodás indoklással." />
      <Segmented
        className="mb-4"
        value={filter}
        onChange={setFilter}
        options={[
          { value: "all", label: "Mind" },
          { value: "actions", label: "Műveletek" },
          { value: "vetoes", label: "Vétók" },
        ]}
      />
      {res.loading ? (
        <LoadingRows rows={5} />
      ) : res.error ? (
        <ErrorState error={res.error} onRetry={res.reload} />
      ) : rows.length === 0 ? (
        <EmptyState title="Nincs bejegyzés" hint="A döntések itt jelennek meg, amint az ügynökök értékelnek." />
      ) : (
        <Card className="divide-y divide-line">
          {rows.map((d) => (
            <DecisionItem key={d.id} d={d} />
          ))}
        </Card>
      )}
    </div>
  );
}
