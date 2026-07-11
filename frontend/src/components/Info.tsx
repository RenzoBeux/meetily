import React from "react";
import { Info as InfoIcon } from "lucide-react";
import { Dialog, DialogContent, DialogTitle, DialogTrigger } from "./ui/dialog";
import { VisuallyHidden } from "./ui/visually-hidden";
import { About } from "./About";

interface InfoProps {
    isCollapsed: boolean;
}

const Info = React.forwardRef<HTMLButtonElement, InfoProps>(({ isCollapsed }, ref) => {
  return (
    <Dialog aria-describedby={undefined}>
      <DialogTrigger asChild>
        <button
          ref={ref}
          className={`flex items-center justify-center mb-2 cursor-pointer border-none transition-colors text-muted-foreground hover:text-foreground ${
            isCollapsed
              ? "bg-transparent p-2 hover:bg-accent rounded-lg"
              : "w-full px-3 py-1.5 mt-1 text-sm font-medium hover:bg-accent rounded-lg"
          }`}
          title="About Murmur"
        >
          <InfoIcon className={isCollapsed ? "w-5 h-5" : "w-4 h-4"} />
          {!isCollapsed && (
            <span className="ml-2 text-sm">About</span>
          )}
        </button>
      </DialogTrigger>
      <DialogContent>
        <VisuallyHidden>
          <DialogTitle>About Murmur</DialogTitle>
        </VisuallyHidden>
        <About />
      </DialogContent>
    </Dialog>
  );
});

Info.displayName = "About";

export default Info; 