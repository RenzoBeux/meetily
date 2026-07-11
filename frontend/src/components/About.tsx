import React, { useState, useEffect } from "react";
import { invoke } from '@tauri-apps/api/core';
import { getVersion } from '@tauri-apps/api/app';
import { Github } from 'lucide-react';
import { Logomark } from './brand/Logomark';

const REPO_URL = 'https://github.com/RenzoBeux/murmur';
const UPSTREAM_URL = 'https://github.com/Zackriya-Solutions/meeting-minutes';

export function About() {
    const [currentVersion, setCurrentVersion] = useState<string>('');

    useEffect(() => {
        // Get current version on mount
        getVersion().then(setCurrentVersion).catch(console.error);
    }, []);

    const openExternal = async (url: string) => {
        try {
            await invoke('open_external_url', { url });
        } catch (error) {
            console.error('Failed to open link:', error);
        }
    };

    return (
        <div className="p-4 space-y-4 h-[80vh] overflow-y-auto">
            {/* Compact Header */}
            <div className="text-center">
                <div className="mb-3 flex justify-center">
                    <Logomark size={64} />
                </div>
                <h1 className="text-xl font-semibold tracking-tight text-foreground">
                    Murmur{currentVersion && <span className="ml-2 text-sm font-normal text-muted-foreground font-mono">v{currentVersion}</span>}
                </h1>
                <p className="text-medium text-muted-foreground mt-1">
                    Real-time notes and summaries that never leave your machine.
                </p>
            </div>

            {/* Features Grid - Compact */}
            <div className="space-y-3">
                <h2 className="text-base font-semibold text-foreground">What makes Murmur different</h2>
                <div className="grid grid-cols-1 sm:grid-cols-2 gap-2">
                    <div className="bg-card border border-border rounded-lg p-3 hover:bg-accent/50 transition-colors">
                        <h3 className="font-bold text-sm text-foreground mb-1">Privacy-first</h3>
                        <p className="text-xs text-muted-foreground leading-relaxed">Your data & AI processing workflow can now stay within your premise. No cloud, no leaks.</p>
                    </div>
                    <div className="bg-card border border-border rounded-lg p-3 hover:bg-accent/50 transition-colors">
                        <h3 className="font-bold text-sm text-foreground mb-1">Use Any Model</h3>
                        <p className="text-xs text-muted-foreground leading-relaxed">Prefer local open-source model? Great. Want to plug in an external API? Also fine. No lock-in.</p>
                    </div>
                    <div className="bg-card border border-border rounded-lg p-3 hover:bg-accent/50 transition-colors">
                        <h3 className="font-bold text-sm text-foreground mb-1">Cost-Smart</h3>
                        <p className="text-xs text-muted-foreground leading-relaxed">Avoid pay-per-minute bills by running models locally (or pay only for the calls you choose).</p>
                    </div>
                    <div className="bg-card border border-border rounded-lg p-3 hover:bg-accent/50 transition-colors">
                        <h3 className="font-bold text-sm text-foreground mb-1">Works everywhere</h3>
                        <p className="text-xs text-muted-foreground leading-relaxed">Google Meet, Zoom, Teams — online or offline.</p>
                    </div>
                </div>
            </div>

            {/* CTA Section - Compact */}
            <div className="text-center space-y-2">
                <p className="text-s text-muted-foreground">
                    Murmur is free and open source. Found a bug or have an idea?
                </p>
                <button
                    onClick={() => openExternal(REPO_URL)}
                    className="inline-flex items-center gap-2 px-4 py-2 bg-primary hover:bg-brand-hover text-primary-foreground text-sm font-medium rounded-md transition-colors duration-200"
                >
                    <Github className="w-4 h-4" />
                    View on GitHub
                </button>
            </div>

            {/* Footer - Compact */}
            <div className="pt-2 border-t border-border text-center space-y-1">
                <p className="text-xs text-muted-foreground/70">
                    Built by Renzo Beux
                </p>
                <p className="text-xs text-muted-foreground/70">
                    Forked from{' '}
                    <button
                        onClick={() => openExternal(UPSTREAM_URL)}
                        className="underline hover:text-muted-foreground"
                    >
                        Meetily
                    </button>{' '}
                    by Zackriya Solutions — thanks for open-sourcing it.
                </p>
            </div>
        </div>

    )
}
