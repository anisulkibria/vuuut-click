/** Vuuut Click browser SDK. No form values, arbitrary query strings, fingerprinting or third-party storage. */
type Engagement = {
    duration_ms: number;
    scroll_percent: number;
};
type Options = {
    endpoint: string;
    consent?: 'required' | 'anonymous';
    identity?: 'session' | 'persistent';
    engagement?: boolean;
};
export declare function createTracker(options: Options): {
    track: (name?: string, vital?: {
        name: string;
        value: number;
        id?: string;
    }, locationOverride?: string, engagement?: Engagement) => void;
    flush: (beacon?: boolean) => Promise<void>;
    trackVital(name: string, value: number, id?: string, page?: string): void;
    identity(): {
        session_id: `${string}-${string}-${string}-${string}-${string}`;
        visitor_id: string | undefined;
    } | null;
    consent(value: boolean): void;
    destroy(): void;
};
declare global {
    interface Window {
        vuuutClick?: ReturnType<typeof createTracker>;
    }
}
export {};
