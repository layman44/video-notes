import { Film } from "lucide-react";
import { useEffect, useState } from "react";

interface VideoThumbnailProps {
  src?: string | null;
  className?: string;
  label?: string;
}

/** Shared cover treatment for places where a source does not provide artwork. */
export function VideoThumbnail({ src, className = "", label = "VIDEO" }: VideoThumbnailProps) {
  const [imageFailed, setImageFailed] = useState(false);
  useEffect(() => setImageFailed(false), [src]);

  return (
    <span className={`video-thumbnail ${className}`}>
      {src && !imageFailed ? (
        <img src={src} alt="" loading="lazy" onError={() => setImageFailed(true)} />
      ) : (
        <span className="video-thumbnail-placeholder" aria-hidden="true">
          <span className="video-thumbnail-orb video-thumbnail-orb-one" />
          <span className="video-thumbnail-orb video-thumbnail-orb-two" />
          <Film size={22} strokeWidth={1.6} />
          <small>{label}</small>
        </span>
      )}
    </span>
  );
}
