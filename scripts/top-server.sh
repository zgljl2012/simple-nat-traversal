#!/usr/bin/env bash

top -pid `ps -ax | grep simple-nat-traversal | grep server | awk '{print $1}'| head -n 1`
