#!/usr/bin/env bash

top -pid `ps -ax | grep simple-nat-traversal | grep client | awk '{print $1}'| head -n 1`
