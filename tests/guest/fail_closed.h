#ifndef FAIL_CLOSED_H
#define FAIL_CLOSED_H

#include <errno.h>
#include <stdio.h>
#include <string.h>
#include <unistd.h>

/*
 * Guest checks for syscall stubs that used to return success.
 * libc's syscall() turns a kernel errno in [-4095, -1] into -1 and errno.
 */

static int __attribute__((unused))
fail_closed_mismatch(const char *what, long rc, int err, int want)
{
	printf("FAIL %s rc=%ld errno=%d (%s) want=%d\n", what, rc, err,
	       strerror(err), want);
	return 1;
}

static int __attribute__((unused))
expect_errno(const char *what, long rc, int want)
{
	int err = errno;
	if (rc != -1 || err != want)
		return fail_closed_mismatch(what, rc, err, want);
	printf("ok %s errno=%d\n", what, err);
	return 0;
}

static int __attribute__((unused))
expect_value(const char *what, long rc, long want)
{
	int err = errno;
	if (rc != want)
		return fail_closed_mismatch(what, rc, err, (int)want);
	printf("ok %s value=%ld\n", what, rc);
	return 0;
}

#endif
