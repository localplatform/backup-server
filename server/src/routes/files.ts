import express, { Request, Response } from 'express';
import fs from 'fs';
import path from 'path';
import { Transform } from 'stream';
import { pipeline } from 'stream/promises';
import { decompress as zstdDecompress } from '@mongodb-js/zstd';
import { logger } from '../utils/logger.js';
import { config } from '../config.js';

// Create a Transform stream for zstd decompression
class ZstdDecompressor extends Transform {
  private chunks: Buffer[] = [];

  _transform(chunk: Buffer, _encoding: string, callback: (error?: Error | null, data?: any) => void) {
    this.chunks.push(chunk);
    callback();
  }

  async _flush(callback: (error?: Error | null, data?: any) => void) {
    try {
      const compressed = Buffer.concat(this.chunks);
      const decompressed = await zstdDecompress(compressed); // await the promise
      this.push(decompressed);
      callback();
    } catch (err) {
      callback(err as Error);
    }
  }
}

const router = express.Router();

interface UploadMetadata {
  jobId: string;
  relativePath: string;
  totalSize: number;
}

/**
 * POST /api/files/upload
 * Receive file uploads from backup agents
 *
 * Headers:
 *   x-job-id: Job ID
 *   x-relative-path: Relative path of the file in backup
 *   x-total-size: Total file size in bytes
 */
router.post('/upload', async (req: Request, res: Response) => {
  try {
    const jobId = req.headers['x-job-id'] as string;
    const relativePath = req.headers['x-relative-path'] as string;
    const totalSize = parseInt(req.headers['x-total-size'] as string, 10);

    if (!jobId || !relativePath || isNaN(totalSize)) {
      return res.status(400).json({
        error: 'Missing required headers: x-job-id, x-relative-path, x-total-size',
      });
    }

    const metadata: UploadMetadata = { jobId, relativePath, totalSize };

    logger.info({ jobId, relativePath, totalSize }, 'Receiving file upload');

    // Determine destination path - use HDD mount point from config
    const baseDir = path.join(config.backupsDir, jobId);
    const destPath = path.join(baseDir, relativePath);

    // Create parent directories
    const destDir = path.dirname(destPath);
    await fs.promises.mkdir(destDir, { recursive: true });

    // Stream the file to disk (with decompression if needed)
    const writeStream = fs.createWriteStream(destPath);
    const contentEncoding = req.headers['content-encoding'];

    try {
      if (contentEncoding === 'zstd') {
        // Decompress zstd stream before writing
        const decompressor = new ZstdDecompressor();
        await pipeline(req, decompressor, writeStream);
      } else {
        // No compression, write directly
        await pipeline(req, writeStream);
      }

      // Verify file size
      const stats = await fs.promises.stat(destPath);
      if (stats.size !== totalSize) {
        logger.warn(
          { jobId, relativePath, expected: totalSize, actual: stats.size },
          'File size mismatch after upload'
        );
        return res.status(400).json({
          error: 'File size mismatch',
          expected: totalSize,
          actual: stats.size,
        });
      }

      logger.info({ jobId, relativePath, size: stats.size }, 'File upload complete');

      res.json({
        success: true,
        path: relativePath,
        size: stats.size,
      });
    } catch (err) {
      logger.error({ jobId, relativePath, err }, 'File upload failed');

      // Clean up partial file
      try {
        await fs.promises.unlink(destPath);
      } catch (unlinkErr) {
        logger.error({ path: destPath, err: unlinkErr }, 'Failed to clean up partial file');
      }

      throw err;
    }
  } catch (err) {
    logger.error({ err }, 'File upload error');
    res.status(500).json({ error: 'File upload failed', message: (err as Error).message });
  }
});

export default router;
