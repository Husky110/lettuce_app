import { BookOpen } from "lucide-react";
import { useImageData } from "../hooks/useImageData";
import { cn } from "../design-tokens";

type LorebookAvatarProps = {
  avatarPath?: string | null;
  name?: string;
  className?: string;
  imageClassName?: string;
  fallbackClassName?: string;
  iconClassName?: string;
};

export function LorebookAvatar({
  avatarPath,
  name,
  className,
  imageClassName,
  fallbackClassName,
  iconClassName,
}: LorebookAvatarProps) {
  const avatarUrl = useImageData(avatarPath);

  if (avatarUrl) {
    return (
      <img
        src={avatarUrl}
        alt={name ? `${name} lorebook image` : "Lorebook image"}
        className={cn("h-full w-full object-cover", className, imageClassName)}
      />
    );
  }

  return (
    <div
      className={cn(
        "flex h-full w-full items-center justify-center bg-linear-to-br from-warning/20 to-warning/80/30",
        className,
        fallbackClassName,
      )}
    >
      <BookOpen className={cn("h-12 w-12 text-warning/80", iconClassName)} />
    </div>
  );
}
