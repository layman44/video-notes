export type ToastType = "success" | "error" | "warning" | "info";

export interface ToastItem {
  id: string;
  type: ToastType;
  message: string;
  duration?: number;
}

type ToastListener = (toasts: ToastItem[]) => void;

class ToastManager {
  private toasts: ToastItem[] = [];
  private listeners: Set<ToastListener> = new Set();

  public subscribe(listener: ToastListener): () => void {
    this.listeners.add(listener);
    listener([...this.toasts]);
    return () => {
      this.listeners.delete(listener);
    };
  }

  private notify() {
    const copy = [...this.toasts];
    for (const listener of this.listeners) {
      listener(copy);
    }
  }

  public show(message: string, type: ToastType = "info", duration = 3200): string {
    const id = Math.random().toString(36).slice(2, 9) + Date.now().toString(36);
    const item: ToastItem = { id, type, message, duration };
    this.toasts = [...this.toasts, item];
    this.notify();

    if (duration > 0) {
      setTimeout(() => {
        this.dismiss(id);
      }, duration);
    }

    return id;
  }

  public dismiss(id: string) {
    this.toasts = this.toasts.filter((t) => t.id !== id);
    this.notify();
  }

  public success(message: string, duration?: number) {
    return this.show(message, "success", duration);
  }

  public error(message: string, duration?: number) {
    return this.show(message, "error", duration ?? 4500);
  }

  public warning(message: string, duration?: number) {
    return this.show(message, "warning", duration);
  }

  public info(message: string, duration?: number) {
    return this.show(message, "info", duration);
  }
}

export const toast = new ToastManager();
