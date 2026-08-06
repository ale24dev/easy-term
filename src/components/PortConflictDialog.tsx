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
          <DialogTitle>Port {port} is in use</DialogTitle>
          <DialogDescription>
            <strong className="text-foreground">{owner.name}</strong> (pid {owner.pid}) is
            already listening on this port.
          </DialogDescription>
        </DialogHeader>
        <DialogFooter>
          <Button type="button" variant="outline" onClick={onCancel} disabled={busy}>
            Cancel
          </Button>
          <Button type="button" onClick={onFreeAndStart} disabled={busy}>
            {busy ? "Freeing…" : "Free and continue"}
          </Button>
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
