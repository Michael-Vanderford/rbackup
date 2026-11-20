import { useMemo } from "react";
import { formatOperationOutput } from "./formatOperationOutput";

export default function OperationOutput({ output, empty }: { output: string; empty: string }) {
    const formatted = useMemo(() => formatOperationOutput(output), [output]);
    return output.trim() ? <>
        <pre tabIndex={0}>{formatted || "No formatted fields. See raw output."}</pre>
        <details><summary>Raw output (original order)</summary><pre tabIndex={0}>{output}</pre></details>
    </> : <p>{empty}</p>;
}
