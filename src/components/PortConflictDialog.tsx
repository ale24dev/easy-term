import type { PortOwner } from "../lib/ipc";

interface PortConflictDialogProps {
  port: number;
  owner: PortOwner;
  busy: boolean;
  onCancel: () => void;
  onFreeAndStart: () => void;
}

export function PortConflictDialog({
  port,
  owner,
  busy,
  onCancel,
  onFreeAndStart,
}: PortConflictDialogProps) {
  return (
    <div className="dialog-backdrop" onClick={onCancel}>
      <div className="dialog" onClick={(e) => e.stopPropagation()}>
        <p className="dialog-title">Puerto {port} ocupado</p>
        <p className="dialog-body">
          <strong>{owner.name}</strong> (pid {owner.pid}) ya está escuchando en este puerto.
        </p>
        <div className="dialog-actions">
          <button type="button" onClick={onCancel} disabled={busy}>
            Cancelar
          </button>
          <button type="button" className="primary" onClick={onFreeAndStart} disabled={busy}>
            {busy ? "Liberando…" : "Liberar y continuar"}
          </button>
        </div>
      </div>
    </div>
  );
}
