// scripts/demo-video/recorder.swift
// Records a single window with ScreenCaptureKit at 60fps H.264.
// usage: recorder <window-id> <out.mov>

import AppKit
import AVFoundation
import CoreMedia
import Foundation
import ScreenCaptureKit

let args = CommandLine.arguments
guard args.count >= 3, let windowID = CGWindowID(args[1]) else {
    FileHandle.standardError.write("usage: recorder <window-id> <out.mov>\n".data(using: .utf8)!)
    exit(2)
}
let outURL = URL(fileURLWithPath: args[2])
_ = NSApplication.shared
try? FileManager.default.removeItem(at: outURL)

final class Recorder: NSObject, SCStreamOutput, SCStreamDelegate {
    let writer: AVAssetWriter
    let input: AVAssetWriterInput
    var started = false
    var lastPTS = CMTime.invalid
    let queue = DispatchQueue(label: "recorder")

    init(width: Int, height: Int) throws {
        writer = try AVAssetWriter(outputURL: outURL, fileType: .mov)
        input = AVAssetWriterInput(mediaType: .video, outputSettings: [
            AVVideoCodecKey: AVVideoCodecType.h264,
            AVVideoWidthKey: width,
            AVVideoHeightKey: height,
            AVVideoCompressionPropertiesKey: [
                AVVideoAverageBitRateKey: 60_000_000,
                AVVideoExpectedSourceFrameRateKey: 60,
                AVVideoProfileLevelKey: AVVideoProfileLevelH264HighAutoLevel,
            ],
        ])
        input.expectsMediaDataInRealTime = true
        writer.add(input)
        super.init()
    }

    func stream(_ stream: SCStream, didOutputSampleBuffer sb: CMSampleBuffer, of type: SCStreamOutputType) {
        guard type == .screen, sb.isValid,
              let attachments = CMSampleBufferGetSampleAttachmentsArray(sb, createIfNecessary: false) as? [[SCStreamFrameInfo: Any]],
              let raw = attachments.first?[.status] as? Int,
              SCFrameStatus(rawValue: raw) == .complete else { return }
        let pts = CMSampleBufferGetPresentationTimeStamp(sb)
        if !started {
            writer.startWriting()
            writer.startSession(atSourceTime: pts)
            started = true
            try? String(format: "%.6f\n", pts.seconds).write(toFile: outURL.path + ".start", atomically: true, encoding: .utf8)
        }
        if lastPTS.isValid && pts <= lastPTS { return }
        if input.isReadyForMoreMediaData {
            input.append(sb)
            lastPTS = pts
        }
    }

    func stream(_ stream: SCStream, didStopWithError error: Error) {
        FileHandle.standardError.write("recorder: stream stopped: \(error)\n".data(using: .utf8)!)
        finish(code: 1)
    }

    func finish(code: Int32) {
        queue.async {
            guard self.started else { exit(code) }
            self.input.markAsFinished()
            self.writer.finishWriting { exit(code) }
        }
    }
}

var recorder: Recorder?
var stream: SCStream?

Task {
    do {
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: true)
        guard let window = content.windows.first(where: { $0.windowID == windowID }) else {
            FileHandle.standardError.write("recorder: window \(windowID) not found\n".data(using: .utf8)!)
            exit(1)
        }
        let filter = SCContentFilter(desktopIndependentWindow: window)
        let scale = Int(filter.pointPixelScale.rounded())
        let cfg = SCStreamConfiguration()
        cfg.width = Int(window.frame.width) * scale
        cfg.height = Int(window.frame.height) * scale
        cfg.minimumFrameInterval = CMTime(value: 1, timescale: 60)
        cfg.pixelFormat = kCVPixelFormatType_32BGRA
        cfg.showsCursor = true
        cfg.queueDepth = 8
        cfg.ignoreShadowsSingleWindow = true
        if #available(macOS 15.0, *) { cfg.showMouseClicks = true }
        let rec = try Recorder(width: cfg.width, height: cfg.height)
        let s = SCStream(filter: filter, configuration: cfg, delegate: rec)
        try s.addStreamOutput(rec, type: .screen, sampleHandlerQueue: rec.queue)
        try await s.startCapture()
        recorder = rec
        stream = s
        print("recording \(cfg.width)x\(cfg.height)")
        fflush(stdout)
    } catch {
        FileHandle.standardError.write("recorder: \(error)\n".data(using: .utf8)!)
        exit(1)
    }
}

var _signalSources: [DispatchSourceSignal] = []
for sig in [SIGINT, SIGTERM] {
    signal(sig, SIG_IGN)
    let src = DispatchSource.makeSignalSource(signal: sig, queue: .main)
    src.setEventHandler {
        Task {
            try? await stream?.stopCapture()
            recorder?.finish(code: 0)
        }
    }
    src.resume()
    _signalSources.append(src)
}
dispatchMain()
