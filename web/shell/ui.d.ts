import { type ButtonHTMLAttributes, type HTMLAttributes, type InputHTMLAttributes, type ReactNode, type SelectHTMLAttributes, type TableHTMLAttributes } from "react";
export declare function Button({ type, className, ...props }: ButtonHTMLAttributes<HTMLButtonElement>): import("react").JSX.Element;
export declare function IconButton({ "aria-label": label, ...props }: ButtonHTMLAttributes<HTMLButtonElement>): import("react").JSX.Element;
export declare function TextField({ className, onInvalid, onInput, ...props }: InputHTMLAttributes<HTMLInputElement>): import("react").JSX.Element;
export declare function Select({ className, onInvalid, onChange, ...props }: SelectHTMLAttributes<HTMLSelectElement>): import("react").JSX.Element;
export declare function Checkbox(props: Omit<InputHTMLAttributes<HTMLInputElement>, "type">): import("react").JSX.Element;
export type DialogProps = {
    title: string;
    description?: string;
    children: ReactNode;
    onClose: () => void;
};
/** Native modal semantics supply focus containment, background inertness and Escape. */
export declare function Dialog({ title, description, children, onClose }: DialogProps): import("react").JSX.Element;
export declare function ConfirmDangerDialog({ title, description, onConfirm, onClose, pending, children }: {
    title: string;
    description?: string;
    onConfirm: () => void;
    onClose: () => void;
    pending?: boolean;
    children?: ReactNode;
}): import("react").JSX.Element;
export declare function Toast({ className, ...props }: HTMLAttributes<HTMLDivElement>): import("react").JSX.Element;
export declare function StatusBadge({ status }: {
    status: string;
}): import("react").JSX.Element;
export declare function Table({ className, ...props }: TableHTMLAttributes<HTMLTableElement>): import("react").JSX.Element;
export declare function FormField({ label, children }: {
    label: string;
    children: ReactNode;
}): import("react").JSX.Element;
export declare function EmptyState({ children }: {
    children: ReactNode;
}): import("react").JSX.Element;
export declare function RequestId({ value }: {
    value?: string | null;
}): import("react").JSX.Element | null;
export declare function ErrorState({ children, requestId, onRetry }: {
    children: ReactNode;
    requestId?: string | null;
    onRetry?: () => void;
}): import("react").JSX.Element;
export declare function LoadingState({ children }: {
    children?: ReactNode;
}): import("react").JSX.Element;
export declare function PageHeader({ children }: {
    children: ReactNode;
}): import("react").JSX.Element;
