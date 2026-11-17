// SPDX-License-Identifier: GPL-2.0-or-later
//
// Copyright 2026 nico359
//
// OpenCV exposure fusion for the three-frame HDR burst.  The frames are
// downscaled before alignment and merging; only the fused result is upscaled
// back to the captured size, keeping the working set well below what a
// full-resolution Mertens merge needs.

#include "camera_c.h"

#include <opencv2/core.hpp>
#include <opencv2/imgcodecs.hpp>
#include <opencv2/imgproc.hpp>
#include <opencv2/photo.hpp>

#include <algorithm>
#include <cmath>
#include <cstdio>
#include <vector>

extern "C" {

int fc2_hdr_fuse(const char* const* paths, int count, const char* output,
                 int working_long_edge)
{
    if (!paths || count < 2 || !output || working_long_edge <= 0)
        return 0;

    try {
        cv::Size full_size;
        cv::Size working_size;
        std::vector<cv::Mat> frames;
        frames.reserve((size_t)count);

        for (int i = 0; i < count; ++i) {
            if (!paths[i])
                return 0;

            cv::Mat image = cv::imread(paths[i], cv::IMREAD_COLOR);
            if (image.empty()) {
                std::fprintf(stderr, "HDR: could not decode %s\n", paths[i]);
                return 0;
            }

            if (full_size.empty()) {
                full_size = image.size();
                const double scale = std::min(
                    1.0, (double)working_long_edge /
                             (double)std::max(full_size.width, full_size.height));
                working_size = cv::Size(
                    std::max(1, (int)std::lround(full_size.width * scale)),
                    std::max(1, (int)std::lround(full_size.height * scale)));
            } else if (image.size() != full_size) {
                std::fprintf(stderr, "HDR: bracket dimensions differ\n");
                return 0;
            }

            cv::Mat reduced;
            cv::resize(image, reduced, working_size, 0, 0, cv::INTER_AREA);
            frames.push_back(reduced);
        }

        std::vector<cv::Mat> aligned;
        cv::Ptr<cv::AlignMTB> aligner = cv::createAlignMTB();
        aligner->process(frames, aligned);
        frames.clear();

        cv::Mat fused;
        cv::Ptr<cv::MergeMertens> merger = cv::createMergeMertens(1.2f, 1.0f, 0.8f);
        merger->process(aligned, fused);
        aligned.clear();

        cv::Mat fused8;
        fused.convertTo(fused8, CV_8U, 255.0);
        fused.release();

        cv::Mat result;
        cv::resize(fused8, result, full_size, 0, 0, cv::INTER_LANCZOS4);
        fused8.release();

        std::vector<int> params = {cv::IMWRITE_JPEG_QUALITY, 95};
        if (!cv::imwrite(output, result, params)) {
            std::fprintf(stderr, "HDR: could not write %s\n", output);
            return 0;
        }

        std::fprintf(stderr, "HDR: OpenCV fusion %dx%d -> %dx%d\n",
                     working_size.width, working_size.height,
                     full_size.width, full_size.height);
        return 1;
    } catch (const cv::Exception& error) {
        std::fprintf(stderr, "HDR: OpenCV exception: %s\n", error.what());
        return 0;
    }
}

} // extern "C"
