import { AlertCircle, AlertTriangle, CheckCircle2, Info, X } from "lucide-react";
import { useEffect, useState } from "react";
import { toast, type ToastItem } from "../lib/toast";

export function ToastContainer() {
  const [toasts, setToasts] = useState<ToastItem[]>([]);

  useEffect(() => {
    return toast.subscribe(setToasts);
  }, []);

  if (toasts.length === 0) return null;

  return (
    <div className="toast-container" role="region" aria-label="通知提示">
      {toasts.map((item) => (
        <div key={item.id} className={`toast-item toast-${item.type}`} role="alert">
          <span className="toast-icon" aria-hidden="true">
            {item.type === "success" ? (
              <CheckCircle2 size={17} />
            ) : item.type === "error" ? (
              <AlertCircle size={17} />
            ) : item.type === "warning" ? (
              <AlertTriangle size={17} />
            ) : (
              <Info size={17} />
            )}
          </span>
          <span className="toast-message">{item.message}</span>
          <button
            type="button"
            className="toast-close"
            onClick={() => toast.dismiss(item.id)}
            aria-label="关闭提示"
          >
            <X size={14} />
          </button>
        </div>
      ))}
    </div>
  );
}
