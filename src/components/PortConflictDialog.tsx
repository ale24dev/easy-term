import type { PortOwner } from "../lib/ipc";
import { Button } from "./ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "./ui/dialog";

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
    <Dialog open onOpenChange={(open) => !open && !busy && onCancel()}>
      <DialogContent showClose={!busy}>
        <DialogHeader>
          <DialogTitle>Puerto {port} ocupado</DialogTitle>
          <DialogDescription>
            <strong className="text-foreground">{owner.name}</strong> (pid {owner.pid}) ya está
            escuchando en este puerto.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={onCancel} disabled={busy}>
            Cancelar
          </Button>
          <Button type="button" onClick={onFreeAndStart} disabled={busy}>
            {busy ? "Liberando…" : "Liberar y continuar"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
