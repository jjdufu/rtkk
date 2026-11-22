import { useMemo, type ReactNode } from "react";
import { Checkbox, type CheckboxCheckedState } from "./Checkbox";

export interface DataTableColumn<T> {
  key: string;
  header: string;
  align?: "left" | "right";
  tabularNums?: boolean;
  widthClass?: string;
  render: (row: T) => ReactNode;
}

export interface DataTableSelection<T> {
  selectedKeys: ReadonlySet<string>;
  onChangeSelectedKeys: (nextKeys: Set<string>) => void;
  isRowSelectable?: (row: T) => boolean;
}

export interface DataTableProps<T> {
  rows: readonly T[];
  getRowKey: (row: T) => string;
  columns: readonly DataTableColumn<T>[];
  selection?: DataTableSelection<T>;
  emptyText?: string;
  emptyAction?: ReactNode;
}

export function DataTable<T>({
  rows,
  getRowKey,
  columns,
  selection,
  emptyText = "暂无记录",
  emptyAction,
}: DataTableProps<T>) {
  const selectableVisibleKeys = useMemo(() => {
    if (!selection) {
      return [] as string[];
    }
    return rows
      .filter((r) => (selection.isRowSelectable ? selection.isRowSelectable(r) : true))
      .map((r) => getRowKey(r));
  }, [rows, selection, getRowKey]);

  const headerCheckState = useMemo<CheckboxCheckedState>(() => {
    if (!selection || selectableVisibleKeys.length === 0) {
      return false;
    }
    let selectedVisibleCount = 0;
    for (const k of selectableVisibleKeys) {
      if (selection.selectedKeys.has(k)) {
        selectedVisibleCount += 1;
      }
    }
    if (selectedVisibleCount === 0) {
      return false;
    }
    if (selectedVisibleCount === selectableVisibleKeys.length) {
      return true;
    }
    return "indeterminate";
  }, [selection, selectableVisibleKeys]);

  const handleToggleAllVisible = (nextChecked: boolean) => {
    if (!selection) {
      return;
    }
    const next = new Set(selection.selectedKeys);
    if (nextChecked) {
      for (const k of selectableVisibleKeys) {
        next.add(k);
      }
    } else {
      for (const k of selectableVisibleKeys) {
        next.delete(k);
      }
    }
    selection.onChangeSelectedKeys(next);
  };

  const handleToggleRow = (rowKey: string, nextChecked: boolean) => {
    if (!selection) {
      return;
    }
    const next = new Set(selection.selectedKeys);
    if (nextChecked) {
      next.add(rowKey);
    } else {
      next.delete(rowKey);
    }
    selection.onChangeSelectedKeys(next);
  };

  if (rows.length === 0) {
    return (
      <div
        className="py-10 px-4 text-center border"
        style={{
          backgroundColor: "var(--surface)",
          borderColor: "var(--border)",
          borderRadius: "var(--radius-md)",
        }}
      >
        <p className="text-[13px] font-medium mb-3" style={{ color: "var(--text-muted)" }}>
          {emptyText}
        </p>
        {emptyAction ? <div className="inline-flex">{emptyAction}</div> : null}
      </div>
    );
  }

  return (
    <div
      className="w-full overflow-x-auto border"
      style={{
        backgroundColor: "var(--bg)",
        borderColor: "var(--border)",
        borderRadius: "var(--radius-md)",
      }}
    >
      <table className="w-full border-collapse text-left text-[13px]">
        <thead>
          <tr
            className="border-b"
            style={{
              backgroundColor: "var(--surface)",
              borderColor: "var(--border)",
            }}
          >
            {selection ? (
              <th className="w-10 px-3 py-2.5">
                <Checkbox
                  checked={headerCheckState}
                  disabled={selectableVisibleKeys.length === 0}
                  ariaLabel="全选可见行"
                  onChange={handleToggleAllVisible}
                />
              </th>
            ) : null}
            {columns.map((col) => (
              <th
                key={col.key}
                className={`px-3 py-2.5 text-xs font-semibold whitespace-nowrap ${
                  col.align === "right" ? "text-right" : "text-left"
                } ${col.widthClass ?? ""}`}
                style={{ color: "var(--text-muted)" }}
              >
                {col.header}
              </th>
            ))}
          </tr>
        </thead>
        <tbody>
          {rows.map((row) => {
            const rowKey = getRowKey(row);
            const rowSelected = selection ? selection.selectedKeys.has(rowKey) : false;
            const rowSelectable =
              selection && selection.isRowSelectable ? selection.isRowSelectable(row) : true;
            return (
              <tr
                key={rowKey}
                className="border-b last:border-b-0 transition-colors"
                style={{
                  borderColor: "var(--border)",
                  backgroundColor: rowSelected ? "var(--surface)" : "transparent",
                }}
              >
                {selection ? (
                  <td className="w-10 px-3 py-2.5 align-middle">
                    <Checkbox
                      checked={rowSelected}
                      disabled={!rowSelectable}
                      ariaLabel={`勾选 ${rowKey}`}
                      onChange={(checked) => handleToggleRow(rowKey, checked)}
                    />
                  </td>
                ) : null}
                {columns.map((col) => (
                  <td
                    key={col.key}
                    className={`px-3 py-2.5 align-middle ${
                      col.align === "right" ? "text-right" : "text-left"
                    } ${col.tabularNums ? "tabular-nums" : ""} ${col.widthClass ?? ""}`}
                    style={{ color: "var(--text)" }}
                  >
                    {col.render(row)}
                  </td>
                ))}
              </tr>
            );
          })}
        </tbody>
      </table>
    </div>
  );
}
