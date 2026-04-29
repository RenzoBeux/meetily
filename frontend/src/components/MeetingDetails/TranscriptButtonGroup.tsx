"use client";

import { useState, useCallback } from 'react';
import { Button } from '@/components/ui/button';
import { ButtonGroup } from '@/components/ui/button-group';
import { Copy, FolderOpen, RefreshCw, Download, Users } from 'lucide-react';
import Analytics from '@/lib/analytics';
import { RetranscribeDialog } from './RetranscribeDialog';
import { ExportMarkdownDialog } from './ExportMarkdownDialog';
import { DiarizeDialog } from './DiarizeDialog';
import { ExportScope } from '@/hooks/meeting-details/useExportOperations';
import { useConfig } from '@/contexts/ConfigContext';


interface TranscriptButtonGroupProps {
  transcriptCount: number;
  onCopyTranscript: () => void;
  onOpenMeetingFolder: () => Promise<void>;
  onExportMarkdown: (scope: ExportScope) => Promise<void>;
  hasSummary: boolean;
  meetingId?: string;
  meetingFolderPath?: string | null;
  onRefetchTranscripts?: () => Promise<void>;
}


export function TranscriptButtonGroup({
  transcriptCount,
  onCopyTranscript,
  onOpenMeetingFolder,
  onExportMarkdown,
  hasSummary,
  meetingId,
  meetingFolderPath,
  onRefetchTranscripts,
}: TranscriptButtonGroupProps) {
  const { betaFeatures } = useConfig();
  const [showRetranscribeDialog, setShowRetranscribeDialog] = useState(false);
  const [showExportDialog, setShowExportDialog] = useState(false);
  const [showDiarizeDialog, setShowDiarizeDialog] = useState(false);

  const handleRetranscribeComplete = useCallback(async () => {
    if (onRefetchTranscripts) {
      await onRefetchTranscripts();
    }
  }, [onRefetchTranscripts]);

  const handleDiarizeComplete = useCallback(async () => {
    if (onRefetchTranscripts) {
      await onRefetchTranscripts();
    }
  }, [onRefetchTranscripts]);

  return (
    <div className="flex items-center justify-center w-full gap-2">
      <ButtonGroup>
        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            Analytics.trackButtonClick('copy_transcript', 'meeting_details');
            onCopyTranscript();
          }}
          disabled={transcriptCount === 0}
          title={transcriptCount === 0 ? 'No transcript available' : 'Copy Transcript'}
        >
          <Copy />
          <span className="hidden lg:inline">Copy</span>
        </Button>

        <Button
          variant="outline"
          size="sm"
          onClick={() => {
            Analytics.trackButtonClick('open_export_dialog', 'meeting_details');
            setShowExportDialog(true);
          }}
          disabled={transcriptCount === 0 && !hasSummary}
          title={transcriptCount === 0 && !hasSummary ? 'Nothing to export yet' : 'Export to Markdown'}
        >
          <Download />
          <span className="hidden lg:inline">Export</span>
        </Button>

        <Button
          size="sm"
          variant="outline"
          className="xl:px-4"
          onClick={() => {
            Analytics.trackButtonClick('open_recording_folder', 'meeting_details');
            onOpenMeetingFolder();
          }}
          title="Open Recording Folder"
        >
          <FolderOpen className="xl:mr-2" size={18} />
          <span className="hidden lg:inline">Recording</span>
        </Button>

        {betaFeatures.importAndRetranscribe && meetingId && meetingFolderPath && (
          <Button
            size="sm"
            variant="outline"
            className="bg-gradient-to-r from-blue-50 to-purple-50 hover:from-blue-100 hover:to-purple-100 border-blue-200 xl:px-4"
            onClick={() => {
              Analytics.trackButtonClick('enhance_transcript', 'meeting_details');
              setShowRetranscribeDialog(true);
            }}
            title="Retranscribe to enhance your recorded audio"
          >
            <RefreshCw className="xl:mr-2" size={18} />
            <span className="hidden lg:inline">Enhance</span>
          </Button>
        )}

        {betaFeatures.speakerDiarization && meetingId && meetingFolderPath && transcriptCount > 0 && (
          <Button
            size="sm"
            variant="outline"
            className="xl:px-4"
            onClick={() => setShowDiarizeDialog(true)}
            title="Identify individual speakers in this meeting"
          >
            <Users className="xl:mr-2" size={18} />
            <span className="hidden lg:inline">Diarize</span>
          </Button>
        )}
      </ButtonGroup>

      <ExportMarkdownDialog
        open={showExportDialog}
        onOpenChange={setShowExportDialog}
        hasSummary={hasSummary}
        hasTranscripts={transcriptCount > 0}
        onExport={onExportMarkdown}
      />

      {betaFeatures.importAndRetranscribe && meetingId && meetingFolderPath && (
        <RetranscribeDialog
          open={showRetranscribeDialog}
          onOpenChange={setShowRetranscribeDialog}
          meetingId={meetingId}
          meetingFolderPath={meetingFolderPath}
          onComplete={handleRetranscribeComplete}
        />
      )}

      {betaFeatures.speakerDiarization && meetingId && (
        <DiarizeDialog
          open={showDiarizeDialog}
          onOpenChange={setShowDiarizeDialog}
          meetingId={meetingId}
          onComplete={handleDiarizeComplete}
        />
      )}
    </div>
  );
}
