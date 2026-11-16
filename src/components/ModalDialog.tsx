import { useEffect, useRef, type ReactNode } from 'react';
import { X } from 'lucide-react';
import { useI18n } from '../lib/useI18n';

interface ModalDialogProps {
  open: boolean;
  onClose: () => void;
  title: string;
  subtitle?: string;
  titleId: string;
  closeOnBackdrop?: boolean;
  maxWidth?: string;
  className?: string;
  children: ReactNode;
}

/**
 * Accessible top-layer dialog built on native `<dialog>.showModal()`.
 * Includes WebKitGTK/Safari fallback for light-dismiss (`closedby="any"`).
 */
export function ModalDialog({
  open,
  onClose,
  title,
  subtitle,
  titleId,
  closeOnBackdrop = true,
  maxWidth = '520px',
  className = '',
  children,
}: ModalDialogProps) {
  const { t } = useI18n();
  const ref = useRef<HTMLDialogElement>(null);
  const pressedOutsideRef = useRef(false);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;

    if (open && !dialog.open) {
      dialog.returnValue = '';
      dialog.showModal();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }, [open]);

  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;

    const handleClose = () => {
      dialog.returnValue = '';
      onClose();
    };

    const isOutside = (e: PointerEvent) => {
      const r = dialog.getBoundingClientRect();
      return (
        e.clientX < r.left ||
        e.clientX > r.right ||
        e.clientY < r.top ||
        e.clientY > r.bottom
      );
    };

    const onPointerDown = (e: PointerEvent) => {
      pressedOutsideRef.current = closeOnBackdrop && isOutside(e);
    };
    const onPointerUp = (e: PointerEvent) => {
      if (
        closeOnBackdrop &&
        !('closedBy' in HTMLDialogElement.prototype) &&
        pressedOutsideRef.current &&
        isOutside(e)
      ) {
        dialog.close();
      }
      pressedOutsideRef.current = false;
    };

    dialog.addEventListener('close', handleClose);
    dialog.addEventListener('pointerdown', onPointerDown);
    dialog.addEventListener('pointerup', onPointerUp);
    return () => {
      dialog.removeEventListener('close', handleClose);
      dialog.removeEventListener('pointerdown', onPointerDown);
      dialog.removeEventListener('pointerup', onPointerUp);
    };
  }, [closeOnBackdrop, onClose]);

  const closedByProps = closeOnBackdrop ? ({ closedby: 'any' } as Record<string, string>) : {};

  return (
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      className={`hs-dialog ${className}`.trim()}
      style={{ maxWidth }}
      {...closedByProps}
    >
      <div className="hs-dialog-header">
        <div className="hs-dialog-heading">
          <h2 id={titleId} className="hs-dialog-title">
            {title}
          </h2>
          {subtitle && (
            <div className="hs-dialog-subtitle" title={subtitle}>
              {subtitle}
            </div>
          )}
        </div>
        <button
          type="button"
          className="icon-btn"
          aria-label={t.common.closeDialog}
          onClick={() => ref.current?.close()}
        >
          <X size={16} aria-hidden="true" />
        </button>
      </div>
      <div className="hs-dialog-body">{children}</div>
    </dialog>
  );
}
